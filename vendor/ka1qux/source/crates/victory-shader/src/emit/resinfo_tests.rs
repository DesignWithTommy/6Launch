//! What a size query gives for a mip level past the last: 0 in every channel,
//! as in D3D12's definition (the reference device does it), where the host
//! driver answers with the size of the last level.
use super::host::{Dispatch, Host, Root, View};
use super::intersect_tests::build_compute_shader;
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use windows::Win32::Graphics::Direct3D12::*;

/// `image_get_resinfo` of a cube with seven levels at levels 0 to 6 and past
/// the last: width, height and level count (dmask x, y, w) of each.
#[test]
fn a_size_query_past_the_last_level_gives_zero() {
    const SIZE: usize = 64;
    const MIPS: u16 = 7;
    let mut words = vec![
        0xBE81_0380,
        0xBE83_0380, // s1, s3 = 0 (upper halves of the tables)
        0xF40C_0400,
        0xFA00_0000, // cube SRV to s[16:23] from s[0:1]
        0xF408_0300,
        0xFA00_0020, // raw output UAV to s[12:15] from s[0:1]+32
        0xF408_0601,
        0xFA00_0000, // sampler to s[24:27] from s[2:3]
        0x7E1E_0280, // v15 = 0 for output byte offsets
    ];
    let levels_asked = [0u32, 1, 3, 5, 6, 7, 8, 100, u32::MAX];
    for (i, &level) in levels_asked.iter().enumerate() {
        // v_mov_b32 v0, level
        words.extend([0x7E00_02FF, level]);
        // MIMG, cube, dmask x, y, w, level in v0, SRV s16, results into v10..v12.
        words.extend([0xF000_0000 | (0x0E << 18) | (0xB << 8) | (3 << 3), 0x00C4_0A00]);
        for channel in 0..3u32 {
            // buffer_store_dword v(10 + channel), v15, s[12:15], 0 offen offset:...
            let at = (i as u32 * 3 + channel) * 4;
            words.extend([0xE070_1000 | at, 0x8003_000F | ((10 + channel) << 8)]);
        }
    }
    words.push(0xBF81_0000); // s_endpgm
    let table = |slot, user_data| RootParam {
        root: RootType::Table,
        table_offset_dwords: 0,
        sgprs: 1,
        base_register: 0,
        user_data,
        slot,
    };
    let range = |kind, at, words, slot, user_data| UsedResource {
        kind,
        root: RootType::Table,
        table_offset_dwords: at,
        lower_bound: 0,
        stride: 0,
        dwords: words,
        user_data,
        slot,
    };
    let native = NativeProgram {
        stage: HwStage::Cs,
        wave64: false,
        vgprs: 16,
        layout: vec![table(0, 0), table(1, 2)],
        used: vec![
            range(ResourceType::Srv, 0, 8, 0, 0),
            range(ResourceType::Uav, 8, 4, 0, 0),
            range(ResourceType::Sampler, 0, 4, 1, 2),
        ],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 3 << 1,
            threads: [1, 1, 1],
        }),
        code: words.into_iter().flat_map(u32::to_le_bytes).collect(),
    };
    let (shader, _) = build_compute_shader(&native).unwrap();
    let texels: Vec<_> = (0..6)
        .flat_map(|_| {
            (0..MIPS).flat_map(|mip| std::iter::repeat_n([f32::from(mip); 4], (SIZE >> mip).pow(2)))
        })
        .collect();
    let sampler = D3D12_SAMPLER_DESC {
        Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        MaxLOD: f32::MAX,
        MaxAnisotropy: 1,
        ..Default::default()
    };
    let expected: Vec<u32> = levels_asked
        .iter()
        .flat_map(|&level| {
            let size = if level < u32::from(MIPS) {
                (SIZE >> level) as u32
            } else {
                0
            };
            [size, size, if size == 0 { 0 } else { u32::from(MIPS) }]
        })
        .collect();
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for (name, warp) in [("WARP", true), ("hardware", false)] {
        let Ok(host) = Host::new(warp) else {
            eprintln!("no SM 6.6 {name} device; skipped");
            continue;
        };
        let (root, pso) = host.compute_pipeline(&shader).unwrap();
        let out = host
            .dispatch(
                &root,
                &pso,
                &Dispatch {
                    roots: vec![(0, Root::Constants(vec![0; shader.user_data_dwords() as usize]))],
                    views: &[
                        View::CubeMips(SIZE as u32, MIPS, &texels),
                        View::RawUav(expected.len() as u32),
                    ],
                    samplers: &[sampler],
                    groups: [1, 1, 1],
                },
            )
            .unwrap();
        // (width, height, levels) of each query.
        assert_eq!(out[0].as_slice(), expected.as_slice(), "{name}");
        ran += 1;
    }
    if ran == 0 {
        eprintln!("no device: the sizes were not checked");
    }
}
