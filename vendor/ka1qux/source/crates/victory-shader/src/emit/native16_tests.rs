//! Native 16-bit float arithmetic ([`super::host_native16`], module notes
//! "16-bit floats"): synthetic programs of the console's f16 instructions,
//! lowered for the f32 path and natively, run on every adapter with
//! native 16-bit operations (the hardware adapter and WARP), one input set
//! per thread. Sums, differences and products must be rounded once to
//! nearest even, fused multiply-adds once where the host fuses its 16-bit
//! `FMad`; minima, maxima, medians and the clamp must be the f32 path's.
//! After upstream VIctory's pull request 2825 by 0xkn0wh3r3 (its tests
//! use a probe harness this tree does not have; these are written anew).

use super::host::{Dispatch, Host, Root, Stride, View};
use super::{HostShader, USER_GROUP_BASE, lower, validator};
use crate::native::{
    ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};
use crate::{io, ir, resource, structure};

const S_ENDPGM: u32 = 0xBF81_0000;
const THREADS: u32 = 64;

const fn s_mov_b32(sdst: u8, src0: u8) -> u32 {
    0xBE80_0000 | (sdst as u32) << 16 | 0x03 << 8 | src0 as u32
}

/// SMEM `s_load_dwordx4 s[dst:dst+3], s[base:base+1], offset`.
const fn s_load_dwordx4(dst: u8, base: u8, offset: u32) -> [u32; 2] {
    [
        0x3D << 26 | 0x02 << 18 | (dst as u32) << 6 | (base as u32 >> 1),
        125 << 25 | offset,
    ]
}

const fn v(n: u8) -> u16 {
    256 + n as u16
}

/// VOP1 `op vdst, src0`.
const fn vop1(op: u16, vdst: u8, src0: u16) -> u32 {
    0x7E00_0000 | (vdst as u32) << 17 | (op as u32) << 9 | (src0 as u32 & 0x1FF)
}

/// VOP2 `op vdst, src0, vsrc1`.
const fn vop2(op: u8, vdst: u8, src0: u16, vsrc1: u8) -> u32 {
    (op as u32 & 0x3F) << 25 | (vdst as u32) << 17 | (vsrc1 as u32) << 9 | (src0 as u32 & 0x1FF)
}

/// VOP3 `op vdst, src0, src1, src2` with `abs`, `neg`, `op_sel` and `clamp`.
const fn vop3(
    op: u16,
    vdst: u8,
    src: [u16; 3],
    abs: u8,
    neg: u8,
    opsel: u8,
    clamp: bool,
) -> [u32; 2] {
    [
        0x35 << 26
            | (op as u32 & 0x3FF) << 16
            | (clamp as u32) << 15
            | (opsel as u32 & 0xF) << 11
            | (abs as u32 & 7) << 8
            | vdst as u32,
        (neg as u32 & 7) << 29
            | (src[0] as u32 & 0x1FF)
            | (src[1] as u32 & 0x1FF) << 9
            | (src[2] as u32 & 0x1FF) << 18,
    ]
}

/// VOP3P `v_pk_* vdst, src0, src1, src2` with `op_sel`, `op_sel_hi`,
/// `neg_lo`, `neg_hi` and `clamp`.
#[allow(clippy::too_many_arguments)]
const fn vop3p(
    op: u8,
    vdst: u8,
    src: [u16; 3],
    opsel: u8,
    opsel_hi: u8,
    neg: u8,
    neg_hi: u8,
    clamp: bool,
) -> [u32; 2] {
    [
        0xCC00_0000
            | (op as u32) << 16
            | (clamp as u32) << 15
            | (opsel_hi as u32 >> 2 & 1) << 14
            | (opsel as u32 & 7) << 11
            | (neg_hi as u32 & 7) << 8
            | vdst as u32,
        (neg as u32 & 7) << 29
            | (opsel_hi as u32 & 3) << 27
            | (src[0] as u32 & 0x1FF)
            | (src[1] as u32 & 0x1FF) << 9
            | (src[2] as u32 & 0x1FF) << 18,
    ]
}

/// MUBUF `op vdata, v8, s[srsrc:srsrc+3], 0 offen offset:offset`.
const fn mubuf(op: u32, vdata: u8, srsrc: u8, offset: u32) -> [u32; 2] {
    [
        0x38 << 26 | op << 18 | 1 << 12 | offset,
        0x80 << 24 | ((srsrc as u32 >> 2) & 0x1F) << 16 | (vdata as u32) << 8 | 8,
    ]
}

/// A program of `THREADS` threads a group: each thread loads its four
/// input dwords into v1..v4, starts v5 and v6 as v4 (the destination's
/// other half), runs `body` and stores v5 and v6.
fn program(body: &[u32]) -> Vec<u32> {
    let mut w = vec![
        s_mov_b32(16, 5), // the group's number in x
        s_mov_b32(5, 0x80),
    ];
    w.extend_from_slice(&s_load_dwordx4(8, 4, 0)); // the output UAV
    w.extend_from_slice(&s_load_dwordx4(12, 4, 0x20)); // the input SRV
    w.push(vop1(0x01, 7, 16)); // v_mov_b32 v7, s16
    w.push(vop2(0x1A, 7, 128 + 6, 7)); // v_lshlrev_b32 v7, 6, v7
    w.push(vop2(0x25, 7, v(0), 7)); // v_add_nc_u32 v7, v0, v7
    w.push(vop2(0x1A, 8, 128 + 4, 7)); // v_lshlrev_b32 v8, 4, v7
    w.extend_from_slice(&mubuf(0x0E, 1, 12, 0)); // buffer_load_dwordx4 v[1:4]
    w.push(vop1(0x01, 5, v(4)));
    w.push(vop1(0x01, 6, v(4)));
    w.extend_from_slice(body);
    w.push(vop2(0x1A, 8, 128 + 3, 7)); // v_lshlrev_b32 v8, 3, v7
    w.extend_from_slice(&mubuf(0x1C, 5, 8, 0)); // buffer_store_dword v5
    w.extend_from_slice(&mubuf(0x1C, 6, 8, 4)); // buffer_store_dword v6, offset:4
    w.push(S_ENDPGM);
    w
}

/// Lowers `words` ([`program`]'s layout) for the f32 path or natively.
fn lower_program(words: &[u32], native16: bool) -> Result<HostShader, String> {
    let view = |kind, table_offset_dwords: u32| UsedResource {
        kind,
        root: RootType::Table,
        table_offset_dwords,
        lower_bound: table_offset_dwords / 8,
        stride: 0,
        dwords: 4,
        user_data: 4,
        slot: 1,
    };
    let native = NativeProgram {
        stage: HwStage::Cs,
        wave64: false,
        vgprs: 16,
        layout: vec![
            RootParam {
                root: RootType::Constants,
                table_offset_dwords: 0,
                sgprs: 4,
                base_register: 0,
                user_data: 0,
                slot: 0,
            },
            RootParam {
                root: RootType::Table,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: 4,
                slot: 1,
            },
        ],
        used: vec![view(ResourceType::Uav, 0), view(ResourceType::Srv, 8)],
        io: StageIo::Cs(ComputeIo {
            // 5 user SGPRs, then the group's number in x.
            rsrc2: 5 << 1 | 1 << 7,
            threads: [THREADS, 1, 1],
        }),
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    };
    let e = |e: crate::ShaderError| e.to_string();
    let p = ir::build(&native.code, native.wave64, native.vgprs).map_err(e)?;
    let st = structure::structure(&p).map_err(e)?;
    let res = resource::track(&p, &native.layout, native.stage, Some(&st)).map_err(e)?;
    let io = io::raise(&p, &native).map_err(e)?;
    let l = lower::with_native16(native16, || {
        lower::lower(&lower::Parts {
            native: &native,
            p: &p,
            st: &st,
            io: &io,
            res: &res,
            inputs: Vec::new(),
            outputs: Vec::new(),
            prims: Vec::new(),
            mesh_stride: 0,
        })
    })
    .map_err(e)?;
    let shader = HostShader {
        stage: HwStage::Cs,
        dxil: l.dxil,
        user_data: l.user_data,
        samplers: l.samplers,
        static_samplers: Vec::new(),
        root_cbvs: l.root_cbvs,
        root_srvs: l.root_srvs,
        root_uavs: l.root_uavs,
        written_root_uavs: l.written_root_uavs,
        writes_guest_memory: l.writes_guest_memory,
        root_indexed_views: res.root_indexed_views,
        half_views: l.half_views,
    };
    if let Ok(mut v) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        v.validate(&mut copy)
            .map_err(|e| format!("DXIL validation: {e}"))?;
    }
    Ok(shader)
}

/// Runs `shader` over `inputs` (one thread each, `times` dispatches back
/// to back): each thread's (v5, v6), and the GPU time of one dispatch.
fn run(
    host: &Host,
    shader: &HostShader,
    inputs: &[[u32; 4]],
    times: u32,
) -> Result<(Vec<[u32; 2]>, f64), String> {
    let groups = inputs.len().div_ceil(THREADS as usize) as u32;
    let mut data: Vec<u32> = inputs.iter().flatten().copied().collect();
    data.resize(groups as usize * THREADS as usize * 4, 0);
    let (root, pso) = host.compute_pipeline(shader)?;
    let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
    for u in &shader.user_data {
        ud[u.index as usize] = match (u.root, u.slot) {
            (RootType::Table, 1) => 0,
            (RootType::Other(USER_GROUP_BASE), _) => 0,
            other => return Err(format!("unexpected user data {other:?}")),
        };
    }
    let views = [
        View::RawUav(groups * THREADS * 2),
        View::Buffer {
            data: &data,
            stride: Stride::Raw,
        },
    ];
    let (out, t) = host.dispatch_timed(
        &root,
        &pso,
        &Dispatch {
            roots: vec![(0, Root::Constants(ud))],
            views: &views,
            samplers: &[],
            groups: [groups, 1, 1],
        },
        times,
    )?;
    let words = &out[0];
    Ok((
        (0..inputs.len())
            .map(|k| [words[2 * k], words[2 * k + 1]])
            .collect(),
        t / f64::from(times),
    ))
}

/// The adapters with native 16-bit operations: (label, WARP, host).
fn native_hosts(adapters: &[(&'static str, bool)]) -> Vec<(&'static str, bool, Host)> {
    adapters
        .iter()
        .filter_map(|&(label, warp)| match Host::new(warp) {
            Ok(h) if h.native16() => Some((label, warp, h)),
            Ok(_) => {
                eprintln!("{label}: no native 16-bit operations; skipped");
                None
            }
            Err(e) => {
                eprintln!("{label}: no SM 6.6 device ({e}); skipped");
                None
            }
        })
        .collect()
}

const BOTH: &[(&str, bool)] = &[("hardware", false), ("warp", true)];

// The model: f16 values as exact integers, results rounded once.

/// The f16 `bits` as (negative, magnitude in units of 2^-24).
fn units(bits: u32) -> (bool, i128) {
    let (e, m) = (bits >> 10 & 0x1F, i128::from(bits & 0x3FF));
    let n = if e == 0 { m } else { (1024 + m) << (e - 1) };
    (bits & 0x8000 != 0, n)
}

fn signed(bits: u32) -> i128 {
    let (neg, n) = units(bits);
    if neg { -n } else { n }
}

/// `n` × 2^-`shift` rounded to an f16 to nearest even; `zero_sign` for an
/// exact zero.
fn round_f16(n: i128, shift: i32, zero_sign: bool) -> u32 {
    if n == 0 {
        return if zero_sign { 0x8000 } else { 0 };
    }
    let sign = if n < 0 { 0x8000 } else { 0 };
    let mag = n.unsigned_abs();
    let top = 127 - mag.leading_zeros() as i32;
    let e = top - shift;
    let step = if e >= -14 { e - 10 } else { -24 };
    let k = step + shift;
    let q = if k <= 0 {
        mag << -k
    } else {
        let q = mag >> k;
        let rem = mag & ((1u128 << k) - 1);
        let half = 1u128 << (k - 1);
        if rem > half || rem == half && q & 1 == 1 {
            q + 1
        } else {
            q
        }
    };
    let bits = (if e >= -14 {
        ((e + 14) as u128) << 10
    } else {
        0
    }) + q;
    sign | if bits >= 0x7C00 { 0x7C00 } else { bits as u32 }
}

fn add16(a: u32, b: u32) -> u32 {
    let both_neg = a & 0x8000 != 0 && b & 0x8000 != 0;
    round_f16(signed(a) + signed(b), 24, both_neg)
}

fn mul16(a: u32, b: u32) -> u32 {
    round_f16(signed(a) * signed(b), 48, (a ^ b) & 0x8000 != 0)
}

/// `a * b + c` rounded once.
fn fma16(a: u32, b: u32, c: u32) -> u32 {
    let p_neg = (a ^ b) & 0x8000 != 0;
    let zero_sign = p_neg && c & 0x8000 != 0;
    round_f16(signed(a) * signed(b) + (signed(c) << 24), 48, zero_sign)
}

/// `a * b + c` rounded to f32 first, then to f16 (the f32 path's fused
/// multiply-add).
fn fma16_twice(a: u32, b: u32, c: u32) -> u32 {
    // Every f16 is an f32 exactly.
    let f = |x: u32| {
        let (neg, n) = units(x);
        let m = (n as f64 * 2f64.powi(-24)) as f32;
        if neg { -m } else { m }
    };
    f32_to_f16(f(a).mul_add(f(b), f(c)))
}

/// An f32 to f16, rounded to nearest even.
fn f32_to_f16(x: f32) -> u32 {
    let bits = x.to_bits();
    let neg = bits >> 31 != 0;
    let (e, m) = ((bits >> 23 & 0xFF) as i32, i128::from(bits & 0x7F_FFFF));
    if e == 0xFF {
        return if neg { 0xFC00 } else { 0x7C00 } | (u32::from(m != 0) * 0x200);
    }
    let (n, shift) = if e == 0 {
        (m, 149)
    } else {
        (m | 1 << 23, 150 - e)
    };
    let n = if neg { -n } else { n };
    // A negative shift (a large number): scale the integer up instead.
    if shift < 0 {
        round_f16(n << -shift, 0, neg)
    } else {
        round_f16(n, shift, neg)
    }
}

fn is_nan(h: u32) -> bool {
    h & 0x7FFF > 0x7C00
}

/// Whether two f16 results agree (any NaN with any NaN).
fn same(a: u32, b: u32) -> bool {
    a == b || is_nan(a) && is_nan(b)
}

/// `n` inputs of finite f16 operands in the low halves (every exponent and
/// sign, denormals included; a fixed sequence), junk in the high halves.
fn random_halves(n: usize, seed: u64) -> Vec<[u32; 4]> {
    let mut x = seed;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    (0..n)
        .map(|_| {
            let mut i = [0u32; 4];
            for k in &mut i {
                let r = next();
                let e = (r >> 16) as u32 % 31;
                *k = (r as u32 & 0x83FF | e << 10) | (r >> 32) as u32 & 0xFFFF_0000;
            }
            i
        })
        .collect()
}

/// f16 values with something to say: zeros, denormals, the extremes of the
/// normals, ones, infinities and NaNs of both signs.
const SPECIAL: [u32; 16] = [
    0x0000, 0x8000, 0x0001, 0x83FF, 0x0400, 0x7BFF, 0xFBFF, 0x3C00, 0xBC00, 0x3800, 0x7C00, 0xFC00,
    0x7E00, 0x7C01, 0xFE00, 0x3555,
];

/// Every pair (and triple, with a third cycling) of [`SPECIAL`], the high
/// halves junk.
fn special_inputs() -> Vec<[u32; 4]> {
    let mut v = Vec::new();
    for (i, &a) in SPECIAL.iter().enumerate() {
        for (j, &b) in SPECIAL.iter().enumerate() {
            let c = SPECIAL[(i * 7 + j * 3) % SPECIAL.len()];
            v.push([
                a | 0x1234_0000,
                b | 0x5678_0000,
                c | 0x9ABC_0000,
                0xDEF0_0000 | (i * 16 + j) as u32,
            ]);
        }
    }
    v
}

/// The low half of `x`, or the high one.
fn half(x: u32, high: bool) -> u32 {
    if high { x >> 16 } else { x & 0xFFFF }
}

/// One case: a body (its result in v5), its inputs, and the result's low
/// half from the inputs (`None`: anything; the high half must stay v4's).
type Case = (&'static str, Vec<u32>, Box<dyn Fn([u32; 4]) -> Option<u32>>);

/// The cases whose native result must be the model's, rounded once to
/// nearest even: sums, differences and products, with modifiers and
/// halves picked by op_sel, and the clamp.
fn rounded_cases() -> Vec<Case> {
    let neg = |x: u32| x ^ 0x8000;
    let abs = |x: u32| x & 0x7FFF;
    let clamp = |x: u32| {
        if x > 0x7C00 {
            0
        } else if x >= 0x3C00 {
            0x3C00
        } else {
            x
        }
    };
    let finite = |x: u32| x & 0x7C00 != 0x7C00;
    // Only finite operands give the model's exact result (an infinite or
    // NaN operand gives the host's NaN or infinity).
    let model =
        move |f: fn(u32, u32) -> u32, a: u32, b: u32| (finite(a) && finite(b)).then(|| f(a, b));
    vec![
        (
            "v_add_f16",
            vec![vop2(0x32, 5, v(1), 2)],
            Box::new(move |i: [u32; 4]| model(add16, half(i[0], false), half(i[1], false))),
        ),
        (
            "v_sub_f16",
            vec![vop2(0x33, 5, v(1), 2)],
            Box::new(move |i: [u32; 4]| model(add16, half(i[0], false), neg(half(i[1], false)))),
        ),
        (
            "v_subrev_f16",
            vec![vop2(0x34, 5, v(1), 2)],
            Box::new(move |i: [u32; 4]| model(add16, half(i[1], false), neg(half(i[0], false)))),
        ),
        (
            "v_mul_f16",
            vec![vop2(0x35, 5, v(1), 2)],
            Box::new(move |i: [u32; 4]| model(mul16, half(i[0], false), half(i[1], false))),
        ),
        (
            "v_add_f16 -|v1.h|, v2 (VOP3, op_sel)",
            vop3(0x132, 5, [v(1), v(2), 0], 1, 1, 1, false).to_vec(),
            Box::new(move |i: [u32; 4]| {
                model(add16, neg(abs(half(i[0], true))), half(i[1], false))
            }),
        ),
        (
            "v_mul_f16 v5.h, v1, -v2 (VOP3, op_sel dst)",
            vop3(0x135, 5, [v(1), v(2), 0], 0, 2, 8, false).to_vec(),
            Box::new(move |i: [u32; 4]| model(mul16, half(i[0], false), neg(half(i[1], false)))),
        ),
        (
            "v_add_f16 clamp",
            vop3(0x132, 5, [v(1), v(2), 0], 0, 0, 0, true).to_vec(),
            Box::new(move |i: [u32; 4]| {
                model(add16, half(i[0], false), half(i[1], false)).map(clamp)
            }),
        ),
    ]
}

/// What a case's v5 must be: the expected half where the instruction
/// writes (the high one with op_sel bit 3), v4's other half.
fn expect(body: &[u32], low: u32, d: u32) -> u32 {
    let high = body.len() == 2 && body[0] >> 26 == 0x35 && body[0] >> 14 & 1 != 0;
    if high {
        d & 0xFFFF | low << 16
    } else {
        d & 0xFFFF_0000 | low
    }
}

#[test]
fn native_f16_sums_and_products_round_once_to_nearest_even() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hosts = native_hosts(BOTH);
    let mut inputs = special_inputs();
    inputs.extend(random_halves(4096, 0x2545_F491_4F6C_DD1D));
    let mut failures = Vec::new();
    for (what, body, model) in rounded_cases() {
        let shader = lower_program(&program(&body), true).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(declares_native16(&shader), "{what}: native");
        // The f32 path for comparison only: its result is the host's
        // conversion's, which need not round to nearest even.
        let f32_path =
            lower_program(&program(&body), false).unwrap_or_else(|e| panic!("{what}: {e}"));
        for (label, _, host) in &hosts {
            let (out, _) =
                run(host, &shader, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            let (ours, _) =
                run(host, &f32_path, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            let (mut checked, mut f32_off) = (0, 0);
            for ((i, r), o) in inputs.iter().zip(&out).zip(&ours) {
                let Some(want) = model(*i) else { continue };
                let want = expect(&body, want, i[3]);
                checked += 1;
                f32_off += usize::from(o[0] != want);
                if r[0] != want && failures.len() < 40 {
                    failures.push(format!(
                        "{what}: {label}: in {i:08x?} want {want:08x} got {:08x}",
                        r[0]
                    ));
                }
            }
            eprintln!(
                "{what}: {label}: the f32 path is off nearest even in {f32_off} of {checked}"
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Packed sums and products, each half on its own, with op_sel picking
/// halves and neg_lo/neg_hi.
#[test]
fn native_packed_f16_sums_and_products_round_once_to_nearest_even() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hosts = native_hosts(BOTH);
    let mut inputs = special_inputs();
    inputs.extend(random_halves(4096, 0x9E37_79B9_7F4A_7C15));
    let finite = |x: u32| x & 0x7C00 != 0x7C00;
    let mut failures = Vec::new();
    // (op, name, f, op_sel, op_sel_hi, neg_lo, neg_hi)
    let cases: [(u8, &str, fn(u32, u32) -> u32, u8, u8, u8, u8); 4] = [
        (0x0F, "v_pk_add_f16", add16, 0, 3, 0, 0),
        (0x10, "v_pk_mul_f16", mul16, 0, 3, 0, 0),
        (
            0x0F,
            "v_pk_add_f16 op_sel:[1,0] op_sel_hi:[0,1] neg_lo:[0,1]",
            add16,
            1,
            2,
            2,
            0,
        ),
        (0x10, "v_pk_mul_f16 neg_hi:[1,0]", mul16, 0, 3, 0, 1),
    ];
    for (op, what, f, opsel, opsel_hi, neg, neg_hi) in cases {
        let body = vop3p(op, 5, [v(1), v(2), 0], opsel, opsel_hi, neg, neg_hi, false).to_vec();
        let shader = lower_program(&program(&body), true).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(declares_native16(&shader), "{what}: native");
        for (label, _, host) in &hosts {
            let (out, _) =
                run(host, &shader, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            for (i, r) in inputs.iter().zip(&out) {
                for (high, sel, sign) in [(false, opsel, neg), (true, opsel_hi, neg_hi)] {
                    let a = half(i[0], sel & 1 != 0) ^ if sign & 1 != 0 { 0x8000 } else { 0 };
                    let b = half(i[1], sel & 2 != 0) ^ if sign & 2 != 0 { 0x8000 } else { 0 };
                    if !(finite(a) && finite(b)) {
                        continue;
                    }
                    let want = f(a, b);
                    let got = half(r[0], high);
                    if got != want && failures.len() < 40 {
                        failures.push(format!(
                            "{what}: {label}: in {i:08x?} half {high} want {want:04x} got {got:04x}"
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `v_fma_f16`, `v_fmac_f16` and `v_pk_fma_f16`: rounded once on the
/// hardware adapter, as the console's; WARP may round in f32 first (the
/// f32 path's result). The first three triples are upstream VIctory's
/// #2384 cases, which tell one rounding from two.
#[test]
fn native_f16_fma_rounds_once_on_the_hardware() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hosts = native_hosts(BOTH);
    const CASES: [[u32; 4]; 3] = [
        [0x3C01, 0x3E00, 0x8001, 0x3E01],
        [0x3C03, 0x3E00, 0x0001, 0x3E05],
        [0x3C01, 0x3E00, 0xBE00, 0x1600],
    ];
    for [a, b, c, want] in CASES {
        assert_eq!(fma16(a, b, c), want, "the model: {a:04x} {b:04x} {c:04x}");
    }
    assert_ne!(fma16_twice(0x3C01, 0x3E00, 0x8001), 0x3E01, "rounded twice");
    let mut inputs: Vec<[u32; 4]> = CASES
        .iter()
        .map(|&[a, b, c, _]| {
            [
                a | 0x3C00_0000,
                b | 0x3C00_0000,
                c | 0x3C00_0000,
                0x5A5A_0000,
            ]
        })
        .collect();
    inputs.extend(random_halves(8192, 0xD1B5_4A32_D192_ED03));
    let mut failures = Vec::new();
    // (name, body, packed, the input whose high half v5 keeps)
    let bodies: [(&str, Vec<u32>, bool, usize); 3] = [
        (
            "v_fma_f16",
            vop3(0x34B, 5, [v(1), v(2), v(3)], 0, 0, 0, false).to_vec(),
            false,
            3,
        ),
        (
            "v_fmac_f16",
            vec![vop1(0x01, 5, v(3)), vop2(0x36, 5, v(1), 2)],
            false,
            2,
        ),
        (
            "v_pk_fma_f16",
            vop3p(0x0E, 5, [v(1), v(2), v(3)], 0, 7, 0, 0, false).to_vec(),
            true,
            0,
        ),
    ];
    for (what, body, packed, kept) in bodies {
        let shader = lower_program(&program(&body), true).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(declares_native16(&shader), "{what}: native");
        for (label, warp, host) in &hosts {
            let (out, _) =
                run(host, &shader, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            let (mut once, mut twice) = (0, 0);
            for (i, r) in inputs.iter().zip(&out) {
                for high in [false, true].into_iter().take(if packed { 2 } else { 1 }) {
                    let (a, b, c) = (half(i[0], high), half(i[1], high), half(i[2], high));
                    // (the high halves of the random inputs are junk: NaNs and
                    // infinities among them)
                    if [a, b, c].iter().any(|&x| x & 0x7C00 == 0x7C00) {
                        continue;
                    }
                    let (want, other) = (fma16(a, b, c), fma16_twice(a, b, c));
                    let got = half(r[0], high);
                    once += usize::from(got == want && want != other);
                    twice += usize::from(got == other && want != other);
                    let ok = got == want || *warp && got == other;
                    if !ok && failures.len() < 40 {
                        failures.push(format!(
                            "{what}: {label}: in {i:08x?} half {high} once {want:04x} twice {other:04x} got {got:04x}"
                        ));
                    }
                }
                if !packed && r[0] >> 16 != i[kept] >> 16 && failures.len() < 40 {
                    failures.push(format!(
                        "{what}: {label}: the high half {:08x} of {:08x}",
                        r[0], i[kept]
                    ));
                }
            }
            eprintln!(
                "{what}: {label}: where one and two roundings differ, once {once}, twice {twice}"
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Minima, maxima, medians (with NaN, both zeros and denormals) and the
/// clamp natively are the f32 path's results, on each adapter, both
/// lowered for it (NaN as any NaN; the clamp of a -0 is +0 natively). Of
/// two zeros, the native median orders -0 below +0, as the minima and
/// maxima do on both paths; the f32 path's median takes the host's
/// `FMin`/`FMax`, which on WARP give either zero (the Radeon orders them).
#[test]
fn native_f16_minima_maxima_and_medians_are_the_f32_paths() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hosts = native_hosts(BOTH);
    let mut inputs = special_inputs();
    inputs.extend(random_halves(1024, 0x0123_4567_89AB_CDEF));
    let bodies: Vec<(&str, Vec<u32>)> = vec![
        ("v_max_f16", vec![vop2(0x39, 5, v(1), 2)]),
        ("v_min_f16", vec![vop2(0x3A, 5, v(1), 2)]),
        (
            "v_max3_f16",
            vop3(0x354, 5, [v(1), v(2), v(3)], 0, 0, 0, false).to_vec(),
        ),
        (
            "v_min3_f16",
            vop3(0x351, 5, [v(1), v(2), v(3)], 0, 0, 0, false).to_vec(),
        ),
        (
            "v_med3_f16",
            vop3(0x357, 5, [v(1), v(2), v(3)], 0, 0, 0, false).to_vec(),
        ),
        (
            "v_med3_f16 -v1, |v2|, v3.h clamp",
            vop3(0x357, 5, [v(1), v(2), v(3)], 2, 1, 4, true).to_vec(),
        ),
        (
            "v_pk_max_f16",
            vop3p(0x12, 5, [v(1), v(2), 0], 0, 3, 0, 0, false).to_vec(),
        ),
        (
            "v_pk_min_f16 neg_lo:[1,0]",
            vop3p(0x11, 5, [v(1), v(2), 0], 0, 3, 1, 0, false).to_vec(),
        ),
    ];
    let mut failures = Vec::new();
    for (what, body) in bodies {
        let median = what.starts_with("v_med3");
        let words = program(&body);
        let ours = lower_program(&words, false).unwrap_or_else(|e| panic!("{what}: {e}"));
        let native = lower_program(&words, true).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(declares_native16(&native), "{what}: native");
        for (label, _, host) in &hosts {
            let (a, _) =
                run(host, &ours, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            let (b, _) =
                run(host, &native, &inputs, 1).unwrap_or_else(|e| panic!("{what}: {label}: {e}"));
            for ((i, x), y) in inputs.iter().zip(&a).zip(&b) {
                for high in [false, true] {
                    let (x, y) = (half(x[0], high), half(y[0], high));
                    // The f32 path's saturate may keep a -0 the console clamps to +0.
                    let alike =
                        same(x, y) || x == 0x8000 && y == 0 || median && (x | y) & 0x7FFF == 0;
                    if !alike && failures.len() < 40 {
                        failures.push(format!("{what}: {label}: in {i:08x?} half {high} f32 path {x:04x} native {y:04x}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Whether a lowered program declares native 16-bit operations (`SFI0`).
fn declares_native16(shader: &HostShader) -> bool {
    let c = crate::dxbc::Container::parse(&shader.dxil).expect("a container");
    let sfi0 = c.part(crate::dxbc::FourCC::SFI0).expect("SFI0");
    u64::from_le_bytes(sfi0.data[..8].try_into().expect("8 bytes")) & 0x4_0000 != 0
}

/// `s_setreg_imm32_b32 hwreg(MODE, 2, 2), mode`: the 16/64-bit round mode.
const fn round_mode(mode: u32) -> [u32; 2] {
    [0xBA80_0881, mode]
}

/// Native 16-bit operations are declared exactly where `half` is used:
/// natively, for the f16 arithmetic; not for a program that only converts,
/// nor for arithmetic under the toward-zero mode (which `half` cannot
/// round), and again once the program goes back to nearest even. Every
/// module validates (where `dxil.dll` is found).
#[test]
fn native_16bit_operations_are_declared_where_used() {
    let add = vec![vop2(0x32, 5, v(1), 2)];
    let pk_fma = vop3p(0x0E, 5, [v(1), v(2), v(3)], 0, 7, 0, 0, false).to_vec();
    let cvt = vec![vop1(0x0A, 5, v(1))];
    let mut omod = vop3(0x132, 5, [v(1), v(2), 0], 0, 0, 0, false).to_vec();
    omod[1] |= 1 << 27; // mul:2: stays on the f32 path
    let mut rtz = round_mode(3).to_vec();
    rtz.push(vop2(0x32, 5, v(1), 2));
    let mut back = rtz.clone();
    back.extend(round_mode(0));
    back.push(vop2(0x32, 6, v(1), 2));
    for (what, body, native) in [
        ("v_add_f16", &add, true),
        ("v_pk_fma_f16", &pk_fma, true),
        ("v_cvt_f16_f32", &cvt, false),
        ("v_add_f16 mul:2", &omod, false),
        ("toward zero", &rtz, false),
        ("toward zero, then nearest even", &back, true),
    ] {
        let words = program(body);
        let f32_path = lower_program(&words, false).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(!declares_native16(&f32_path), "{what}: the f32 path");
        let shader = lower_program(&words, true).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(declares_native16(&shader), native, "{what}");
    }
}

/// The toward-zero program keeps its sum in f32 natively too (the same
/// results as the f32 path), and the sum after going back to nearest even
/// is rounded once to nearest even.
#[test]
fn native_f16_results_round_as_the_mode_says() {
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let hosts = native_hosts(BOTH);
    let mut body = round_mode(3).to_vec();
    body.push(vop2(0x32, 5, v(1), 2));
    body.extend(round_mode(0));
    body.push(vop2(0x32, 6, v(1), 2));
    let words = program(&body);
    let ours = lower_program(&words, false).expect("f32 path");
    let native = lower_program(&words, true).expect("native");
    let inputs = random_halves(2048, 0x5851_F42D_4C95_7F2D);
    let mut failures = Vec::new();
    for (label, _, host) in &hosts {
        let (a, _) = run(host, &ours, &inputs, 1).expect("f32 path");
        let (b, _) = run(host, &native, &inputs, 1).expect("native");
        for ((i, x), y) in inputs.iter().zip(&a).zip(&b) {
            let want = i[3] & 0xFFFF_0000 | add16(half(i[0], false), half(i[1], false));
            if (x[0] != y[0] || y[1] != want) && failures.len() < 40 {
                failures.push(format!(
                    "{label}: in {i:08x?} f32 path {x:08x?} native {y:08x?} want {want:08x}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What native 16-bit arithmetic saves: dependent chains of the console's
/// f16 instructions, lowered for the f32 path and natively, timed on the
/// hardware adapter (GPU timestamps around ten back-to-back dispatches,
/// the best of five), at two chain lengths: the difference is the chains'
/// own cost, without the loads and stores around them. After upstream
/// VIctory's pull request 2806.
/// `cargo test -p victory-shader --release --lib translated_f16_math -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark"]
fn translated_f16_math_costs() {
    const GROUPS: usize = 4096;
    const TIMES: u32 = 10;
    let _one = super::valu_tests::ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Some((label, _, host)) = native_hosts(&[("hardware", false)]).into_iter().next() else {
        return;
    };
    let chain = |step: &[u32], times: usize| -> Vec<u32> {
        let mut b = vec![vop1(0x01, 5, v(1))];
        for _ in 0..times {
            b.extend_from_slice(step);
        }
        b
    };
    let pk_fma = vop3p(0x0E, 5, [v(5), v(2), v(3)], 0, 7, 0, 0, false);
    let fma = vop3(0x34B, 5, [v(5), v(2), v(3)], 0, 0, 0, false);
    let mul_add = [vop2(0x35, 5, v(5), 2), vop2(0x32, 5, v(5), 3)];
    // (name, body at a chain length, f16 operations per thread per instruction)
    let cases: [(&str, &dyn Fn(usize) -> Vec<u32>, usize); 3] = [
        ("v_pk_fma_f16", &|n| chain(&pk_fma, n), 2),
        ("v_fma_f16", &|n| chain(&fma, n), 1),
        ("v_mul_f16 + v_add_f16", &|n| chain(&mul_add, n / 2), 1),
    ];
    const SHORT: usize = 512;
    const LONG: usize = 4096;
    // Halves near 1 so the chains neither overflow nor vanish.
    let threads = THREADS as usize * GROUPS;
    let mut x = 0x9E37_79B9u32;
    let mut near_one = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        0x3800 | x & 0x03FF
    };
    let inputs: Vec<[u32; 4]> = (0..threads)
        .map(|_| {
            let (a, b, c) = (near_one(), near_one(), near_one() ^ 0x8000);
            [a | a << 16, b | b << 16, c | c << 16, 0]
        })
        .collect();
    for (what, body, per) in cases {
        let mut chains = Vec::new();
        for native16 in [false, true] {
            let mut times = Vec::new();
            for n in [SHORT, LONG] {
                let shader = lower_program(&program(&body(n)), native16).expect("lowered");
                let mut best = f64::MAX;
                let mut out = Vec::new();
                for _ in 0..5 {
                    let (o, t) = run(&host, &shader, &inputs, TIMES).expect("dispatch");
                    best = best.min(t);
                    out = o;
                }
                times.push((best, out));
            }
            // The cost of LONG - SHORT instructions a thread.
            let chain = times[1].0 - times[0].0;
            let rate = (threads * per * (LONG - SHORT)) as f64 / chain / 1e9;
            eprintln!(
                "{label} {what:>22} {:>6}: {SHORT}: {:8.3} ms, {LONG}: {:8.3} ms; the chain {:8.3} ms, {rate:6.0} G f16 ops/s",
                if native16 { "native" } else { "f32" },
                times[0].0 * 1e3,
                times[1].0 * 1e3,
                chain * 1e3
            );
            chains.push((chain, times.pop().expect("two lengths").1));
        }
        let alike = chains[0]
            .1
            .iter()
            .zip(&chains[1].1)
            .filter(|(a, b)| a[0] == b[0])
            .count();
        eprintln!(
            "{label} {what:>22}  speed-up of the chain {:.1}x; results alike in {alike} of {threads} threads",
            chains[0].0 / chains[1].0
        );
    }
}
