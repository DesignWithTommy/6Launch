//! Series S presentation: the title presents two planes, the 1080p world and a
//! 4K UI plane drawn over it (menus, loading art, the brightness symbol's
//! frame). The console's display blends them; here a compute pass of ours
//! does: the UI plane (premultiplied alpha) over the world plane, both scaled
//! to one 1080p image, which is then presented like a single plane.
//!
//! From Linox-IDK/LNX-victory's `compose` (GPL-3.0), without its gamma switch
//! and with the plane choice (`VICTORY_PRESENT_PLANE`) kept.

use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::s;

use crate::host::{HostList, host, transition};
use crate::object::This;

const SOURCE: &str = r#"
#define RS "DescriptorTable(SRV(t0, numDescriptors=2), UAV(u0)), StaticSampler(s0, filter=FILTER_MIN_MAG_MIP_LINEAR, addressU=TEXTURE_ADDRESS_CLAMP, addressV=TEXTURE_ADDRESS_CLAMP, addressW=TEXTURE_ADDRESS_CLAMP)"
Texture2D<float4> world : register(t0);
Texture2D<float4> ui : register(t1);
RWTexture2D<float4> output : register(u0);
SamplerState linear_clamp : register(s0);
[RootSignature(RS)]
[numthreads(8, 8, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint w, h;
    output.GetDimensions(w, h);
    if (id.x >= w || id.y >= h) return;
    float2 uv = (float2(id.xy) + 0.5) / float2(w, h);
    float3 g = world.SampleLevel(linear_clamp, uv, 0).rgb;
    float4 u = ui.SampleLevel(linear_clamp, uv, 0);
    float3 c = saturate(u.rgb + g * (1.0 - u.a));
    output[id.xy] = float4(c, 1.0);
}
"#;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

struct Compose {
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    heap: ID3D12DescriptorHeap,
    increment: usize,
    output: ID3D12Resource,
    list: HostList,
}

// SAFETY: D3D12 objects are free-threaded; the list is used under `LOCK`.
unsafe impl Send for Compose {}
// SAFETY: as above.
unsafe impl Sync for Compose {}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The pass's compute shader, compiled (with its root signature inside).
fn compile() -> windows::core::Result<ID3DBlob> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source and names live through the call; out pointers ours.
    let compiled = unsafe {
        D3DCompile(
            SOURCE.as_ptr().cast(),
            SOURCE.len(),
            s!("compose"),
            None,
            None,
            s!("main"),
            s!("cs_5_1"),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = compiled {
        if let Some(errors) = errors {
            // SAFETY: the blob's own bytes.
            let text = unsafe {
                std::slice::from_raw_parts(
                    errors.GetBufferPointer().cast::<u8>(),
                    errors.GetBufferSize(),
                )
            };
            tracing::warn!("plane composition: {}", String::from_utf8_lossy(text));
        }
        return Err(e);
    }
    code.ok_or_else(windows::core::Error::empty)
}

fn compose() -> Option<&'static Compose> {
    static C: std::sync::OnceLock<Option<Compose>> = std::sync::OnceLock::new();
    C.get_or_init(|| match make() {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!(error = %e, "no plane composition: one plane is shown alone");
            None
        }
    })
    .as_ref()
}

fn make() -> windows::core::Result<Compose> {
    let host = host().ok_or_else(windows::core::Error::empty)?;
    let device = &host.device;
    let code = compile()?;
    // SAFETY: plain object creation; the blob's own bytes (root signature inside).
    unsafe {
        let bytes =
            std::slice::from_raw_parts(code.GetBufferPointer().cast::<u8>(), code.GetBufferSize());
        let root: ID3D12RootSignature = device.CreateRootSignature(0, bytes)?;
        let pso: ID3D12PipelineState =
            device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::transmute_copy(&root),
                CS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: bytes.as_ptr().cast(),
                    BytecodeLength: bytes.len(),
                },
                ..Default::default()
            })?;
        let heap: ID3D12DescriptorHeap =
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 3,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })?;
        let increment = device
            .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
            as usize;
        let mut output: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: u64::from(WIDTH),
                Height: HEIGHT,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                ..Default::default()
            },
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            None,
            &mut output,
        )?;
        let output = output.ok_or_else(windows::core::Error::empty)?;
        let mut cpu = heap.GetCPUDescriptorHandleForHeapStart();
        cpu.ptr += 2 * increment;
        device.CreateUnorderedAccessView(
            &output,
            None,
            Some(&D3D12_UNORDERED_ACCESS_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
                ..Default::default()
            }),
            cpu,
        );
        let list = host.new_list()?;
        list.close();
        tracing::info!("Series S planes: composed into one 1080p image");
        Ok(Compose {
            root,
            pso,
            heap,
            increment,
            output,
            list,
        })
    }
}

/// A shader view of a plane's host texture, written at `slot`.
fn view(c: &Compose, resource: This, texture: &ID3D12Resource, slot: usize) {
    let Some(host) = host() else { return };
    // SAFETY: a live resource; our heap.
    unsafe {
        let desc = texture.GetDesc();
        let format =
            crate::resource::view_format(resource).map_or(desc.Format, |f| DXGI_FORMAT(f as i32));
        let mut cpu = c.heap.GetCPUDescriptorHandleForHeapStart();
        cpu.ptr += slot * c.increment;
        host.device.CreateShaderResourceView(
            texture,
            Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: format,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MipLevels: 1,
                        ..Default::default()
                    },
                },
            }),
            cpu,
        );
    }
}

/// `VICTORY_PRESENT_PLANE=n`: the plane shown alone, as the title numbers
/// them (the last one is a clamp), in place of the composition.
pub(crate) fn picked_plane() -> Option<u32> {
    static PICK: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *PICK.get_or_init(|| {
        std::env::var("VICTORY_PRESENT_PLANE")
            .ok()
            .and_then(|v| v.trim().parse().ok())
    })
}

/// Whether `planes` presented planes are composed: Series S (whose title
/// presents the world and the UI apart), at least two, and no plane picked.
pub(crate) fn wanted(planes: u32) -> bool {
    planes >= 2 && crate::console::current() == crate::console::Console::SeriesS && picked_plane().is_none()
}

/// Blends `ui` over `world` (title resources with host copies) and presents
/// the result as frame `frame`. False if it could not (the caller shows a
/// single plane instead).
pub(crate) fn present(world: This, ui: This, frame: u64) -> bool {
    let (Some(c), Some(host)) = (compose(), host()) else {
        return false;
    };
    let (Some(w_tex), Some(u_tex)) = (
        crate::resource::host_resource(world),
        crate::resource::host_resource(ui),
    ) else {
        return false;
    };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    const READ: u32 = 0x40; // non-pixel shader resource
    let w_state = crate::resource::host_transition(world, READ).map_or(READ, |(_, before)| before);
    let u_state = crate::resource::host_transition(ui, READ).map_or(READ, |(_, before)| before);
    view(c, world, w_tex, 0);
    view(c, ui, u_tex, 1);
    c.list.reset();
    let l = &c.list.list;
    let flip = |t: &ID3D12Resource, from: u32, to: u32| {
        (from != to).then(|| {
            transition(
                t,
                D3D12_RESOURCE_STATES(from as i32),
                D3D12_RESOURCE_STATES(to as i32),
            )
        })
    };
    // SAFETY: an open list; our objects and the planes' live host copies.
    unsafe {
        let before: Vec<D3D12_RESOURCE_BARRIER> = [flip(w_tex, w_state, READ), flip(u_tex, u_state, READ)]
            .into_iter()
            .flatten()
            .collect();
        if !before.is_empty() {
            l.ResourceBarrier(&before);
        }
        l.SetDescriptorHeaps(&[Some(c.heap.clone())]);
        l.SetComputeRootSignature(&c.root);
        l.SetPipelineState(&c.pso);
        l.SetComputeRootDescriptorTable(0, c.heap.GetGPUDescriptorHandleForHeapStart());
        l.Dispatch(WIDTH.div_ceil(8), HEIGHT.div_ceil(8), 1);
        let after: Vec<D3D12_RESOURCE_BARRIER> = [flip(w_tex, READ, w_state), flip(u_tex, READ, u_state)]
            .into_iter()
            .flatten()
            .collect();
        if !after.is_empty() {
            l.ResourceBarrier(&after);
        }
    }
    c.list.close();
    host.execute(&[&c.list]);
    crate::resource::host_transition(world, w_state);
    crate::resource::host_transition(ui, u_state);
    const UNORDERED: u32 = 0x8;
    host.present(&c.output, UNORDERED, frame).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pass's source compiles, with the root signature it declares.
    #[test]
    fn the_composition_shader_compiles() {
        let code = compile().expect("the shader compiles");
        // SAFETY: the blob's own size.
        assert!(unsafe { code.GetBufferSize() } > 0);
    }

    #[test]
    fn two_planes_are_composed_on_series_s_only() {
        // Series X is the default in tests.
        assert!(!wanted(2));
        assert!(!wanted(1));
    }
}
