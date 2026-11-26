//! Buffer memory shared by the waves of one thread group across `s_barrier`, as
//! `compute_graph_ops:CS_FormulaOperation` shares its stages' results: a typed
//! 16-bit UAV store in one stage, `s_waitcnt_vscnt` and `s_barrier`, then
//! another wave's typed load of that word in the next stage. Run on the host GPU.
//!
//! On an RX 9070 XT (RADV, Mesa 26.2) about 70,000 of the 163,840 reads were stale before
//! the translation gave such programs an LDS word (`lower::Lower::one_cu_zero`): RADV runs a
//! compute group without LDS across both CUs of a WGP, and nothing invalidated the other
//! CU's L0 at the barrier. The reader is 9 waves away (576 threads): with an even distance
//! (512) the two waves ran on one CU and every read was fresh.

use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32_TYPELESS};

use super::host::{Dispatch, Host, Root, View};
use super::{HostShader, lower, validator};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{io, ir, resource, structure};

const THREADS: u32 = 1024;
const ROUNDS: u32 = 8;

fn mubuf_idxen(op: u32, vdata: u32, vaddr: u32, srsrc: u32) -> [u32; 2] {
    [0x38 << 26 | op << 18 | 1 << 13, 0x80 << 24 | (srsrc >> 2) << 16 | vdata << 8 | vaddr]
}

/// The program: thread t reads the word of thread t + 576 (another wave, of the other parity, half
/// the group away), writes its own word, waits for its stores and the group,
/// reads the other word again and counts the rounds that did not see the
/// other thread's new value; the count goes to dword t of a raw UAV.
fn program(lds: bool) -> NativeProgram {
    const D16_X_LOAD: u32 = 0x80;
    const D16_XYZW_STORE: u32 = 0x87;
    const WAIT_VM: u32 = 0xBF8C_3F70; // s_waitcnt vmcnt(0)
    const WAIT_LGKM: u32 = 0xBF8C_C07F; // s_waitcnt lgkmcnt(0)
    let mut w = vec![
        0xBE85_0380, // s_mov_b32 s5, 0
        0x3D << 26 | 0x02 << 18 | 8 << 6 | 2,
        125 << 25, // s_load_dwordx4 s[8:11], s[4:5], 0
        0x3D << 26 | 0x02 << 18 | 12 << 6 | 2,
        125 << 25 | 32, // s_load_dwordx4 s[12:15], s[4:5], 32
        WAIT_LGKM,
        0x4A04_00FF,
        0x240, // v_add_nc_u32 v2, 0x240, v0 (9 waves on: the other CU of a WGP if waves alternate)
        0x3604_04FF,
        0x3FF, // v_and_b32 v2, 0x3ff, v2
        0x7E0A_0280, // v_mov_b32 v5, 0
        0xBE94_0380, // s_mov_b32 s20, 0
    ];
    if lds {
        // LDS[t] = 0 (never read): a program that uses LDS (which the host runs in CU mode)
        w.extend([0x3412_0082, 0x7E14_0280]); // v_lshlrev_b32 v9, 2, v0; v_mov_b32 v10, 0
        w.extend([0x36 << 26 | 0x0D << 18, 10 << 8 | 9]); // ds_write_b32 v9, v10
        w.push(WAIT_LGKM);
    }
    let top = w.len();
    w.extend(mubuf_idxen(D16_X_LOAD, 6, 2, 8)); // buffer_load_format_d16_x v6, v2, s[8:11] (cached)
    w.push(WAIT_VM);
    w.extend([
        0x8015_8114, // s_add_u32 s21, s20, 1
        0x7E06_0215, // v_mov_b32 v3, s21
        0x7E08_0215, // v_mov_b32 v4, s21
    ]);
    w.extend(mubuf_idxen(D16_XYZW_STORE, 3, 0, 8)); // buffer_store_format_d16_xyzw v[3:4], v0, s[8:11]
    w.extend([
        WAIT_VM,
        0xBBFD_0000, // s_waitcnt_vscnt null, 0
        0xBF8A_0000, // s_barrier
    ]);
    w.extend(mubuf_idxen(D16_X_LOAD, 6, 2, 8)); // buffer_load_format_d16_x v6, v2, s[8:11]
    w.extend([
        WAIT_VM,
        0x360C_0CFF,
        0xFFFF,      // v_and_b32 v6, 0xffff, v6
        0x7D8A_0C15, // v_cmp_ne_u32 vcc, s21, v6
        0x500A_0A80, // v_add_co_ci_u32 v5, vcc, 0, v5, vcc
        0xBF8A_0000, // s_barrier (the next round's store after every read)
        0x8014_8114, // s_add_u32 s20, s20, 1
        0xBF0A_8814, // s_cmp_lt_u32 s20, 8 (ROUNDS)
    ]);
    let back = top as i32 - (w.len() as i32 + 1);
    w.push(0xBF85_0000 | (back as u32 & 0xFFFF)); // s_cbranch_scc1 top
    w.push(0x3412_0082); // v_lshlrev_b32 v9, 2, v0
    w.extend([0x38 << 26 | 0x1C << 18 | 1 << 12, 0x80 << 24 | 3 << 16 | 5 << 8 | 9]); // buffer_store_dword v5, v9, s[12:15] offen
    w.push(0xBF81_0000); // s_endpgm
    let used = |off: u32, stride: u32| UsedResource {
        kind: ResourceType::Uav,
        root: RootType::Table,
        table_offset_dwords: off,
        lower_bound: off / 8,
        stride,
        dwords: 4,
        user_data: 4,
        slot: 0,
    };
    NativeProgram {
        stage: HwStage::Cs,
        wave64: true,
        vgprs: 16,
        layout: vec![RootParam {
            root: RootType::Table,
            table_offset_dwords: 0,
            sgprs: 1,
            base_register: 0,
            user_data: 4,
            slot: 0,
        }],
        used: vec![used(0, 4), used(8, 1)],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1 | u32::from(lds) * (THREADS * 4 / 512) << 15,
            threads: [THREADS, 1, 1],
        }),
        code: w.iter().flat_map(|x| x.to_le_bytes()).collect(),
    }
}

fn lowered(native: &NativeProgram) -> HostShader {
    let p = ir::build(&native.code, native.wave64, native.vgprs).expect("ir::build");
    let st = structure::structure(&p).expect("structure");
    let res = resource::track(&p, &native.layout, native.stage, Some(&st)).expect("resource::track");
    let io = io::raise(&p, native).expect("io::raise");
    let lowered = lower::lower(&lower::Parts {
        native,
        p: &p,
        st: &st,
        io: &io,
        res: &res,
        inputs: Vec::new(),
        outputs: Vec::new(),
        prims: Vec::new(),
        mesh_stride: 0,
    })
    .expect("lower");
    let shader = HostShader {
        stage: HwStage::Cs,
        dxil: lowered.dxil,
        user_data: lowered.user_data,
        samplers: lowered.samplers,
        static_samplers: Vec::new(),
        root_cbvs: lowered.root_cbvs,
        root_srvs: lowered.root_srvs,
        root_uavs: lowered.root_uavs,
        root_indexed_views: res.root_indexed_views,
    };
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        v.validate(&mut shader.dxil.clone()).expect("DXIL validation failed");
    }
    shader
}

/// Every round sees the other wave's word written before the barrier.
#[test]
fn a_typed_uav_word_written_before_a_barrier_is_seen_by_another_wave_after_it() {
    assert_eq!(stale_reads(false), 0);
}

fn stale_reads(lds: bool) -> u64 {
    // (`lds`: the program writes LDS itself, never reading it.)
    let shader = lowered(&program(lds));
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(host) = Host::new(false) else {
        eprintln!("no SM 6.6 hardware device; not run");
        return 0;
    };
    let (root, pso) = host.compute_pipeline(&shader).expect("pipeline");
    let words = vec![0u32; THREADS as usize / 2];
    let zeros = vec![0u32; THREADS as usize];
    let views = [
        View::TypedUav { data: &words, format: DXGI_FORMAT_R16_UINT, elem_bytes: 2 },
        View::TypedUav { data: &zeros, format: DXGI_FORMAT_R32_TYPELESS, elem_bytes: 4 },
    ];
    let roots = || {
        let mut roots = vec![(0, Root::Constants(vec![0; shader.user_data_dwords().max(1) as usize]))];
        if !shader.root_srvs.is_empty() {
            let mut strides = vec![2 | super::VIEW_INTEGER, 1];
            strides.resize(256, 0);
            roots.push((1, Root::View { srv: true, data: strides }));
        }
        roots
    };
    let mut stale = 0;
    let mut runs = 0;
    for _ in 0..20 {
        let out = host
            .dispatch(&root, &pso, &Dispatch { roots: roots(), views: &views, samplers: &[], groups: [1, 1, 1] })
            .expect("dispatch");
        stale += out[1].iter().map(|&n| u64::from(n)).sum::<u64>();
        runs += 1;
    }
    eprintln!("lds={lds}: {stale} stale reads in {runs} dispatches of {THREADS} threads x {ROUNDS} rounds");
    stale
}
