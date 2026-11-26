use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use victory_formats::texture::Layout;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

use super::*;
use crate::descriptor::{create_heap, create_placed_srv, heap};
use crate::host::{Host, host};
use crate::object::{S_OK, guid, slot};
use crate::resource::{self, Memory, ResourceDesc};

const SIDE: u32 = 256;
const MIPS: u32 = 4;
const TEXTURES: u32 = 8;
const VIEWS: u32 = 8;
const ROUNDS: u32 = 4;
const RGBA8: u32 = DXGI_FORMAT_R8G8B8A8_UNORM.0 as u32;
const TEXTURE_DESC: [u32; 14] = [
    3,
    0,
    0,
    0,
    SIDE,
    0,
    SIDE,
    MIPS << 16 | 1,
    RGBA8,
    1,
    0,
    0x105,
    0,
    0,
];

struct TitleHeap(This);

impl TitleHeap {
    fn new() -> Self {
        // A shader-visible view heap with room for its optional placement fields.
        let desc = [
            D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV.0 as u32,
            TEXTURES * VIEWS,
            D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE.0 as u32,
            0,
            0,
            0,
            0,
            0,
        ];
        let i = ID3D12DescriptorHeap::IID;
        let iid = guid(i.data1, i.data2, i.data3, i.data4);
        let mut out = null();
        assert_eq!(
            create_heap(null(), desc.as_ptr().cast(), &iid, &mut out),
            S_OK
        );
        Self(out)
    }
}

impl Drop for TitleHeap {
    fn drop(&mut self) {
        let release = slot::<extern "system" fn(This) -> u32>(self.0, 0x10);
        release(self.0);
    }
}

struct Source(Memory);

impl Source {
    fn write(&mut self, layout: &Layout, round: u32, texture: u32) {
        // SAFETY: the source owns the allocation and the previous list finished.
        let tiled = unsafe {
            std::slice::from_raw_parts_mut(self.0.address() as *mut u8, layout.size as usize)
        };
        for mip in 0..MIPS {
            let side = (SIDE >> mip) as usize;
            let pitch = side * 4;
            let linear: Vec<_> = (0..pitch * side)
                .map(|byte| texel_byte(round, texture, mip, byte))
                .collect();
            layout
                .tile(tiled, mip, 0, &linear, pitch, linear.len())
                .unwrap();
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        resource::release_test_placed_texture(self.0.address(), TEXTURE_DESC);
    }
}

fn texel_byte(round: u32, texture: u32, mip: u32, byte: usize) -> u8 {
    (37 * round + 13 * texture + 17 * mip + 3 * byte as u32 + (byte as u32 >> 8)) as u8
}

struct Readback {
    buffer: ID3D12Resource,
    footprints: [D3D12_PLACED_SUBRESOURCE_FOOTPRINT; MIPS as usize],
    size: usize,
}

impl Readback {
    fn record(host: &Host, list: &HostList, source: &Source) -> Self {
        let resource = resource::placed_known(source.0.address(), &TEXTURE_DESC).unwrap();
        let texture = resource::host_resource(resource).unwrap();
        crate::list::ensure_state(list, resource, D3D12_RESOURCE_STATE_COPY_SOURCE.0 as u32);
        let mut footprints = [D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); MIPS as usize];
        let mut size = 0;
        // SAFETY: the texture is live and the output holds all its subresources.
        unsafe {
            host.device.GetCopyableFootprints(
                &texture.GetDesc(),
                0,
                MIPS,
                0,
                Some(footprints.as_mut_ptr()),
                None,
                None,
                Some(&mut size),
            );
        }
        let buffer = host.read_back_buffer(size).unwrap();
        for (mip, footprint) in footprints.iter().enumerate() {
            let from = D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: the copy location borrows the texture without releasing it.
                pResource: unsafe { std::mem::transmute_copy(texture) },
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    SubresourceIndex: mip as u32,
                },
            };
            let to = D3D12_TEXTURE_COPY_LOCATION {
                // SAFETY: as above, for the readback buffer.
                pResource: unsafe { std::mem::transmute_copy(&buffer) },
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: *footprint,
                },
            };
            // SAFETY: the open list names the live source and destination.
            unsafe { list.list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
        }
        Self {
            buffer,
            footprints,
            size: size as usize,
        }
    }

    fn check(&self, round: u32, texture: u32) {
        let mut ptr = null_mut();
        // SAFETY: the list has completed and this buffer is live.
        unsafe { self.buffer.Map(0, None, Some(&mut ptr)).unwrap() };
        // SAFETY: Map succeeded and the buffer holds at least `size` bytes.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), self.size) };
        for (mip, footprint) in self.footprints.iter().enumerate() {
            let side = (SIDE >> mip) as usize;
            let width = side * 4;
            for y in 0..side {
                let at = footprint.Offset as usize + y * footprint.Footprint.RowPitch as usize;
                for (x, &actual) in bytes[at..at + width].iter().enumerate() {
                    let expected = texel_byte(round, texture, mip as u32, y * width + x);
                    assert_eq!(
                        actual, expected,
                        "round {round}, texture {texture}, mip {mip}, row {y}, byte {x}"
                    );
                }
            }
        }
        // SAFETY: the mapped bytes are no longer used.
        unsafe { self.buffer.Unmap(0, None) };
    }
}

#[test]
fn duplicate_texture_views_share_uploads_within_each_pass() {
    let Some(host) = host() else {
        eprintln!("Skipped streaming upload readback: no D3D12 device");
        return;
    };
    // SAFETY: the words have ResourceDesc's layout and contain only integer fields.
    let desc: ResourceDesc = unsafe { std::mem::transmute(TEXTURE_DESC) };
    let layout = resource::texture_layout(&desc)
        .unwrap()
        .planes
        .remove(0)
        .layout;
    let size = resource::placed_bytes(&TEXTURE_DESC).unwrap();
    let mut sources: Vec<_> = (0..TEXTURES)
        .map(|_| Source(Memory::zeroed(size, layout.alignment).unwrap()))
        .collect();
    let title_heap = TitleHeap::new();
    let heap = heap(title_heap.0);
    let host_heap = heap.host().unwrap();
    let state = State::default();
    let uploads_before = HEAP_TEXTURES.load(Ordering::Relaxed);
    let bytes_before = HEAP_TEXTURE_BYTES.load(Ordering::Relaxed);
    let mut peak_staging = 0;
    let mut recording = Duration::ZERO;
    let mut submitting = Duration::ZERO;

    for round in 0..ROUNDS {
        for (texture, source) in sources.iter_mut().enumerate() {
            source.write(&layout, round, texture as u32);
        }
        for index in 0..TEXTURES * VIEWS {
            let source = &sources[((index / VIEWS + round) % TEXTURES) as usize];
            let start = source.0.address();
            // Newest written slots are served first. Some views expose only smaller mips.
            let first = if index % VIEWS < VIEWS / 2 { 0 } else { 2 };
            let view = [
                RGBA8,
                D3D12_SRV_DIMENSION_TEXTURE2D.0 as u32,
                D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                0,
                first,
                MIPS - first,
                0,
                0,
                0,
                0,
                start as u32,
                (start >> 32) as u32,
                0,
                0,
                0,
                0,
            ];
            create_placed_srv(null(), &TEXTURE_DESC, &view, heap.handle(index).unwrap());
        }

        let list = host.new_list().unwrap();
        let before = crate::texture::memory()[1];
        let started = Instant::now();
        PASS_BUDGET.set(Some((5000, 64 << 20)));
        state.heap_textures(&list, heap, host_heap);
        PASS_BUDGET.set(None);
        recording += started.elapsed();
        peak_staging = peak_staging.max(crate::texture::memory()[1].saturating_sub(before));
        assert!((0..TEXTURES * VIEWS).all(|index| host_heap.made(index)));
        assert!(
            host_heap
                .texture_slots_due(crate::frame::current(), 1)
                .0
                .is_empty()
        );
        let readbacks: Vec<_> = sources
            .iter()
            .map(|source| Readback::record(host, &list, source))
            .collect();
        list.close();
        let started = Instant::now();
        host.execute(&[&list]);
        submitting += started.elapsed();
        for (texture, readback) in readbacks.iter().enumerate() {
            readback.check(round, texture as u32);
        }
    }

    let uploads = HEAP_TEXTURES.load(Ordering::Relaxed) - uploads_before;
    let bytes = HEAP_TEXTURE_BYTES.load(Ordering::Relaxed) - bytes_before;
    eprintln!(
        "adapter={:?}, slots={}, staged_uploads={uploads}, staged_title_bytes={bytes}, peak_staging_bytes={peak_staging}, record_us={}, submit_and_wait_us={}",
        crate::host::adapter_summary(),
        ROUNDS * TEXTURES * VIEWS,
        recording.as_micros(),
        submitting.as_micros()
    );
    assert_eq!(uploads, u64::from(ROUNDS * TEXTURES * 2));
}
