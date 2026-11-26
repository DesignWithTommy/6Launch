//! Output tests for console wave64 compute programs on a host of 32-lane waves: tiny RDNA
//! sequences go through the whole translation (`super::emit`, its retries included) for a
//! host whose waves have 32 lanes, run as compute programs on the D3D12 devices there are
//! (WARP, and the default adapter when it is a hardware one), and every thread's results are
//! compared with a CPU model of what the console's 64-lane wave computes. Partial EXEC masks
//! and lanes 32..63 throughout, groups of one and of two console waves.
//!
//! Every program starts `s_mov_b32 s5, 0; s_load_dwordx4 s[8:11], s[4:5], 0` (a raw UAV's
//! descriptor), with the root constants in s0..s3 and the thread index in v0, then
//! `v9 = 4 t` (the thread's dword) and `v10 = 8 t + s0` (a value that differs per lane).

use super::host::{Dispatch, Host, Root, View};
use super::{HostShader, lower, validator};
use crate::dxbc::{Container, FourCC};
use crate::native::{ComputeIo, HwStage, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource};
use crate::{ShaderError, io, resource, structure};

const S_ENDPGM: u32 = 0xBF81_0000;
const VCC: u32 = 106;
const EXEC: u32 = 126;
const LIT: u32 = 255;
const fn k(n: u32) -> u32 {
    128 + n
}
const fn v(n: u32) -> u32 {
    256 + n
}

const fn sop1(sdst: u32, op: u32, ssrc0: u32) -> u32 {
    0xBE80_0000 | sdst << 16 | op << 8 | ssrc0
}
const fn sopp(op: u32, imm: u32) -> u32 {
    0xBF80_0000 | op << 16 | imm
}
const fn vop1(vdst: u32, op: u32, src0: u32) -> u32 {
    0x7E00_0000 | vdst << 17 | op << 9 | src0
}
const fn vop2(op: u32, vdst: u32, src0: u32, vsrc1: u32) -> u32 {
    op << 25 | vdst << 17 | vsrc1 << 9 | src0
}
const fn vopc(op: u32, src0: u32, vsrc1: u32) -> u32 {
    0x7C00_0000 | op << 17 | vsrc1 << 9 | src0
}
const fn vop3(op: u32, vdst: u32, src0: u32, src1: u32, src2: u32) -> [u32; 2] {
    [0xD400_0000 | op << 16 | vdst, src0 | src1 << 9 | src2 << 18]
}
const fn ds(op: u32, vdst: u32, addr: u32, data0: u32, offset: u32) -> [u32; 2] {
    [0xD800_0000 | op << 18 | offset, addr | data0 << 8 | vdst << 24]
}
/// `buffer_store_dword v<vdata>, v<vaddr>, s[8:11], 0 offen offset:<offset>`.
const fn store(vdata: u32, vaddr: u32, offset: u32) -> [u32; 2] {
    [0xE000_0000 | 0x1C << 18 | 1 << 12 | offset, vaddr | vdata << 8 | 2 << 16 | 0x80 << 24]
}
const fn dpp16(vsrc0: u32, ctrl: u32, row_mask: u32, bank_mask: u32, bound_ctrl: bool) -> u32 {
    row_mask << 28 | bank_mask << 24 | (bound_ctrl as u32) << 19 | ctrl << 8 | vsrc0
}

// opcodes
const S_MOV_B32: u32 = 0x03;
const S_MOV_B64: u32 = 0x04;
const S_BCNT1_B64: u32 = 0x10;
const S_FF1_B64: u32 = 0x14;
const S_AND_SAVEEXEC_B64: u32 = 0x24;
const S_CBRANCH_EXECZ: u32 = 0x08;
const V_MOV: u32 = 0x01;
const V_READFIRSTLANE: u32 = 0x02;
const V_LSHLREV: u32 = 0x1A;
const V_AND: u32 = 0x1B;
const V_XOR: u32 = 0x1D;
const V_ADD: u32 = 0x25;
const V_CMP_LT_U32: u32 = 0xC1;
const V_CMP_GT_U32: u32 = 0xC4;
const V_CMP_NE_U32: u32 = 0xC5;
const V_CMPX_LE_U32: u32 = 0xD3;
const V_READLANE: u32 = 0x360;
const V_WRITELANE: u32 = 0x361;
const V_MBCNT_LO: u32 = 0x365;
const V_MBCNT_HI: u32 = 0x366;
const V_PERMLANEX16: u32 = 0x378;
const DS_WRITE_B32: u32 = 0x0D;
const DS_SWIZZLE: u32 = 0x35;
const DS_READ_B32: u32 = 0x36;
const DS_BPERMUTE: u32 = 0xB3;

/// v9 = 4 t, v10 = 8 t + s0, v11 = t & 63 (the console lane).
fn prelude() -> Vec<u32> {
    vec![
        vop2(V_LSHLREV, 9, k(2), 0),
        vop2(V_LSHLREV, 10, k(3), 0),
        vop2(V_ADD, 10, 0, 10),
        vop2(V_AND, 11, k(63), 0),
    ]
}

/// What v10 holds in thread `t`.
fn val(t: u32, s0: u32) -> u32 {
    8 * t + s0
}

/// A wave64 compute program of `threads` threads in a row, `lds` bytes of LDS.
fn native(body: &[u32], threads: u32, lds: u32) -> NativeProgram {
    let mut words = vec![
        0xBE85_0380, // s_mov_b32 s5, 0
        0x3D << 26 | 0x02 << 18 | 8 << 6 | 2,
        125 << 25, // s_load_dwordx4 s[8:11], s[4:5], 0
    ];
    words.extend(prelude());
    words.extend_from_slice(body);
    words.push(S_ENDPGM);
    NativeProgram {
        stage: HwStage::Cs,
        wave64: true,
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
        used: vec![UsedResource {
            kind: ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: 0,
            lower_bound: 0,
            stride: 0,
            dwords: 4,
            user_data: 4,
            slot: 1,
        }],
        io: StageIo::Cs(ComputeIo {
            rsrc2: 5 << 1 | lds.div_ceil(512) << 15,
            threads: [threads, 1, 1],
        }),
        code: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
    }
}

/// The whole translation of `native` for a host of 32-lane waves (`wide`: with the
/// wave-wide branches `VICTORY_WAVE_WIDE=1` allows).
fn translate(native: &NativeProgram, wide: bool) -> Result<HostShader, ShaderError> {
    let c = Container {
        digest: [0; 16],
        version: (1, 0),
        parts: Vec::new(),
    };
    let p = crate::spill::build(native)?;
    let st = structure::structure(&p)?;
    let res = resource::track(&p, &native.layout, native.stage, Some(&st))?;
    let io = io::raise(&p, native)?;
    let was = lower::TEST_HOST_WAVE.replace(Some(32));
    let r = if wide {
        super::with_wave_wide(|| super::emit(&c, native, &p, &st, &io, &res))
    } else {
        super::emit(&c, native, &p, &st, &io, &res)
    };
    lower::TEST_HOST_WAVE.set(was);
    r
}

/// The wave size the translation requires of the host (PSV0's minimum and maximum lanes).
fn wave_size(shader: &HostShader) -> Option<u32> {
    let c = Container::parse(&shader.dxil).ok()?;
    let psv = c.part(FourCC::PSV0)?.data;
    let min = u32::from_le_bytes(psv.get(20..24)?.try_into().ok()?);
    let max = u32::from_le_bytes(psv.get(24..28)?.try_into().ok()?);
    (min == max).then_some(min)
}

/// The devices to run on: WARP, and the default adapter when it is a hardware one; with each
/// its wave lanes (min, max).
fn devices() -> Vec<(&'static str, Host, (u32, u32))> {
    use windows::Win32::Graphics::Direct3D12::{D3D12_FEATURE_D3D12_OPTIONS1, D3D12_FEATURE_DATA_D3D12_OPTIONS1};
    let mut out = Vec::new();
    for (name, warp) in [("WARP", true), ("hardware", false)] {
        let Ok(host) = Host::new(warp) else { continue };
        let mut o = D3D12_FEATURE_DATA_D3D12_OPTIONS1::default();
        // SAFETY: a plain query that fills a structure of the given size.
        let ok = unsafe {
            host.device
                .CheckFeatureSupport(D3D12_FEATURE_D3D12_OPTIONS1, (&raw mut o).cast(), size_of_val(&o) as u32)
                .is_ok()
        };
        let lanes = if ok { (o.WaveLaneCountMin, o.WaveLaneCountMax) } else { (0, 0) };
        out.push((name, host, lanes));
    }
    out
}

/// Runs `shader` with s0, s1 = `consts` over `groups` groups; the UAV's `dwords` dwords.
fn run(host: &Host, shader: &HostShader, consts: [u32; 2], dwords: u32, groups: u32) -> Result<Vec<u32>, String> {
    let (root, pso) = host.compute_pipeline(shader)?;
    let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
    for u in &shader.user_data {
        if let (RootType::Constants, 0, d @ 0..=1) = (u.root, u.slot, u.dword) {
            ud[u.index as usize] = consts[d as usize];
        }
    }
    let out = host.dispatch(
        &root,
        &pso,
        &Dispatch {
            roots: vec![(0, Root::Constants(ud))],
            views: &[View::RawUav(dwords)],
            samplers: &[],
            groups: [groups, 1, 1],
        },
    )?;
    Ok(out.into_iter().next().unwrap_or_default())
}

/// One case: `outs` rows of one dword per thread (row r of thread t at dword r T + t).
struct Case<'a> {
    name: &'a str,
    body: Vec<u32>,
    threads: u32,
    lds: u32,
    outs: u32,
    wide: bool,
}

/// What a case came to.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Every run on every device that took it matched the console model.
    Exact,
    /// Refused by the translation (nothing wrong runs).
    Refused(String),
    /// Translated, but no device took the pipeline.
    NotRun,
    /// Some device's results differ from the console's: (device, the first dwords that do).
    Differs(Vec<(&'static str, String)>),
}

impl Outcome {
    /// Panics unless every device that took the program matched the console.
    #[track_caller]
    fn exact(self, name: &str) {
        match self {
            Outcome::Exact => {}
            // (no device here takes it: a machine without a card of 32-lane waves)
            Outcome::NotRun => eprintln!("{name}: not run"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

/// Translates and runs `case` for each of `consts` on every device, comparing with `model`
/// (s0, s1 -> every dword).
fn check(case: &Case, consts: &[[u32; 2]], model: impl Fn([u32; 2]) -> Vec<u32>) -> Outcome {
    let native = native(&case.body, case.threads, case.lds);
    let shader = match translate(&native, case.wide) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}: refused: {e}", case.name);
            return Outcome::Refused(e.to_string());
        }
    };
    if let Ok(mut vd) = validator::Validator::load(validator::Validator::default_path().as_deref()) {
        let mut copy = shader.dxil.clone();
        vd.validate(&mut copy).unwrap_or_else(|e| panic!("{}: DXIL validation failed: {e}", case.name));
    }
    let size = wave_size(&shader);
    let _one = super::valu_tests::ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = false;
    let mut differs = Vec::new();
    'devices: for (dev, host, lanes) in devices() {
        let dwords = case.outs * case.threads;
        for &c in consts {
            let got = match run(&host, &shader, c, dwords, 1) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("{}: {dev} (wave lanes {lanes:?}) does not take WaveSize {size:?}: {e}", case.name);
                    continue 'devices;
                }
            };
            ran = true;
            let want = model(c);
            let bad: Vec<String> = (0..dwords as usize)
                .filter(|&i| got.get(i) != want.get(i))
                .take(6)
                .map(|i| {
                    let (row, t) = (i as u32 / case.threads, i as u32 % case.threads);
                    format!("row {row} thread {t}: got {:#x}, console {:#x}", got.get(i).copied().unwrap_or(0), want[i])
                })
                .collect();
            if !bad.is_empty() {
                let what = format!("wave lanes {lanes:?}, WaveSize {size:?}, s0 = {:#x}, s1 = {}: {}", c[0], c[1], bad.join("; "));
                eprintln!("{}: {dev} DIFFERS: {what}", case.name);
                differs.push((dev, what));
                continue 'devices;
            }
        }
        eprintln!("{}: {dev} (wave lanes {lanes:?}, WaveSize {size:?}): matches the console", case.name);
    }
    match (ran, differs.is_empty()) {
        (false, _) => Outcome::NotRun,
        (true, true) => Outcome::Exact,
        (true, false) => Outcome::Differs(differs),
    }
}

/// Every thread of `threads`: (thread, console wave, lane).
fn threads(n: u32) -> impl Iterator<Item = (u32, u32, u32)> {
    (0..n).map(|t| (t, t >> 6, t & 63))
}

/// `v_readlane` at a constant lane past 31 and at an SGPR's lane, `v_writelane` at both:
/// every lane needs the other host wave's lane.
#[test]
fn lane_reads_and_writes_across_the_halves() {
    let mut body = Vec::new();
    body.extend(vop3(V_READLANE, 20, v(10), k(45), 0)); // v_readlane_b32 s20, v10, 45
    body.extend(vop3(V_READLANE, 21, v(10), 1, 0)); // v_readlane_b32 s21, v10, s1
    body.push(vop1(3, V_MOV, 20));
    body.push(vop1(4, V_MOV, 21));
    body.push(vop1(5, V_MOV, v(0)));
    body.extend(vop3(V_WRITELANE, 5, 0, k(50), 0)); // v_writelane_b32 v5, s0, 50
    body.extend(vop3(V_WRITELANE, 5, 0, 1, 0)); // v_writelane_b32 v5, s0, s1
    for n in [64u32, 128] {
        let mut b = body.clone();
        b.extend(store(3, 9, 0));
        b.extend(store(4, 9, 4 * n));
        b.extend(store(5, 9, 8 * n));
        let case = Case {
            name: &format!("readlane/writelane, {n} threads"),
            body: b,
            threads: n,
            lds: 0,
            outs: 3,
            wide: false,
        };
        let o = check(&case, &[[7, 3], [7, 37], [100, 63], [1, 0]], |[s0, s1]| {
            let mut out = vec![0; 3 * n as usize];
            for (t, w, l) in threads(n) {
                out[t as usize] = val(64 * w + 45, s0);
                out[(n + t) as usize] = val(64 * w + (s1 & 63), s0);
                out[(2 * n + t) as usize] = if l == 50 || l == s1 & 63 { s0 } else { t };
            }
            out
        });
        o.exact(&case.name);
    }
}

/// `v_readfirstlane` with the lanes below s1 out of EXEC (`v_cmpx`): the console's first lane
/// of EXEC, in whichever half it is. The program has no other cross-lane code, so by default
/// it declares no wave size and each host wave reads its own first lane (documented, not
/// asserted); with `VICTORY_WAVE_FIRSTLANE=1` it is the console's.
#[test]
fn first_lane_reads_take_the_consoles_first_lane() {
    for n in [64u32, 128] {
        let mut body = vec![
            vopc(V_CMPX_LE_U32, 1, 11),  // v_cmpx_le_u32 exec, s1, v11: lanes from s1 on
            vop1(20, V_READFIRSTLANE, v(10)), // v_readfirstlane_b32 s20, v10
            vop1(3, V_MOV, 20),
        ];
        body.extend(store(3, 9, 0));
        for on in [false, true] {
            let case = Case {
                name: &format!("readfirstlane under partial EXEC, {n} threads, VICTORY_WAVE_FIRSTLANE={}", u8::from(on)),
                body: body.clone(),
                threads: n,
                lds: 0,
                outs: 1,
                wide: false,
            };
            let consts = [[5, 0], [5, 1], [5, 31], [5, 32], [5, 33], [9, 50], [9, 63], [9, 64]];
            let was = lower::TEST_FIRSTLANE.replace(Some(on));
            let o = check(&case, &consts, |[s0, s1]| {
                threads(n).map(|(_, w, l)| if l >= s1 { val(64 * w + s1, s0) } else { 0 }).collect()
            });
            lower::TEST_FIRSTLANE.set(was);
            if on {
                o.exact(&case.name);
            } else {
                eprintln!("{}: {o:?}", case.name);
            }
        }
    }
}

/// A ballot into an SGPR pair (`v_cmp_ne_u32 s[20:21]`), then `v_mbcnt_lo/hi`, `s_bcnt1`,
/// `s_ff1` and the high dword read as data.
#[test]
fn ballots_counts_and_mbcnt_follow_the_consoles_64_bits() {
    for n in [64u32, 128] {
        let mut body = vec![vop2(V_AND, 5, 0, 0)]; // v_and_b32 v5, s0, v0
        body.extend(vop3(V_CMP_NE_U32, 20, k(0), v(5), 0)); // v_cmp_ne_u32 s[20:21], 0, v5
        body.extend(vop3(V_MBCNT_LO, 3, 20, k(0), 0));
        body.extend(vop3(V_MBCNT_HI, 3, 21, v(3), 0));
        body.push(sop1(22, S_BCNT1_B64, 20));
        body.push(sop1(23, S_FF1_B64, 20));
        body.push(vop1(4, V_MOV, 22));
        body.push(vop1(6, V_MOV, 23));
        body.push(vop1(7, V_MOV, 21));
        body.extend(store(3, 9, 0));
        body.extend(store(4, 9, 4 * n));
        body.extend(store(6, 9, 8 * n));
        body.extend(store(7, 9, 12 * n));
        let case = Case {
            name: &format!("ballot, v_mbcnt, s_bcnt1, s_ff1, {n} threads"),
            body,
            threads: n,
            lds: 0,
            outs: 4,
            wide: false,
        };
        let o = check(&case, &[[0x20, 0], [0x21, 0], [0x3F, 0], [0, 0], [0x10, 0], [0x42, 0]], |[s0, _]| {
            let mut out = vec![0; 4 * n as usize];
            for (t, w, l) in threads(n) {
                let mask: u64 = (0..64).filter(|&j| (64 * w + j) & s0 != 0).map(|j| 1u64 << j).sum();
                out[t as usize] = (mask & ((1u64 << l) - 1)).count_ones();
                out[(n + t) as usize] = mask.count_ones();
                out[(2 * n + t) as usize] = if mask == 0 { u32::MAX } else { mask.trailing_zeros() };
                out[(3 * n + t) as usize] = (mask >> 32) as u32;
            }
            out
        });
        o.exact(&case.name);
    }
}

/// `s_and_saveexec_b64` with the lanes past s1, a skip when none is left, then EXEC read as
/// data inside: the lane's place among the lanes left and their count (the allocation idiom's
/// numbers).
#[test]
fn saveexec_and_exec_as_data_inside_a_lane_branch() {
    for n in [64u32, 128] {
        let mut inner = Vec::new();
        inner.extend(vop3(V_MBCNT_LO, 3, EXEC, k(0), 0));
        inner.extend(vop3(V_MBCNT_HI, 3, EXEC + 1, v(3), 0));
        inner.push(sop1(20, S_BCNT1_B64, EXEC));
        inner.push(vop1(4, V_MOV, 20));
        inner.extend(store(3, 9, 0));
        inner.extend(store(4, 9, 4 * n));
        let mut body = vec![
            vopc(V_CMP_LT_U32, 1, 11),                // v_cmp_lt_u32 vcc, s1, v11
            sop1(12, S_AND_SAVEEXEC_B64, VCC),        // s_and_saveexec_b64 s[12:13], vcc
            sopp(S_CBRANCH_EXECZ, inner.len() as u32), // past the body
        ];
        body.extend(inner);
        body.push(sop1(EXEC, S_MOV_B64, 12));
        for wide in [false, true] {
            let case = Case {
                name: &format!("saveexec, EXEC as data, {n} threads{}", if wide { ", wave-wide" } else { "" }),
                body: body.clone(),
                threads: n,
                lds: 0,
                outs: 2,
                wide,
            };
            let o = check(&case, &[[0, 0], [0, 20], [0, 31], [0, 32], [0, 40], [0, 62], [0, 63]], |[_, s1]| {
                let mut out = vec![0; 2 * n as usize];
                for (t, _, l) in threads(n) {
                    if l > s1 {
                        out[t as usize] = l - s1 - 1;
                        out[(n + t) as usize] = 63 - s1;
                    }
                }
                out
            });
            o.exact(&case.name);
        }
    }
}

/// The cross-lane operations the ISA keeps inside 32-lane halves – `ds_swizzle` (bit masks
/// and quad mode), DPP16 (`row_shr`, `row_mirror` under a row mask), DPP8 and
/// `v_permlanex16` – with lanes 32..63 reading only their own half. `crossing`: with a
/// `v_readlane` of lane 33 too, so that the two host waves exchange.
#[test]
fn half_wave_cross_lane_operations_keep_to_their_half() {
    let dpp8_reverse: u32 = (0..8).map(|i| (7 - i) << (3 * i)).sum();
    for crossing in [false, true] {
        for n in [64u32, 128] {
            let mut body = vec![vop1(3, V_MOV, k(0)), vop1(6, V_MOV, k(0))];
            body.extend(ds(DS_SWIZZLE, 4, 10, 0, 0x1F | 0x11 << 10)); // lane ^ 17 in each 32
            body.extend(ds(DS_SWIZZLE, 5, 10, 0, 0x8000 | 0x1B)); // quad reversed
            body.extend([vop1(3, V_MOV, 0xFA), dpp16(10, 0x111, 0xF, 0xF, false)]); // row_shr:1
            body.extend([vop1(6, V_MOV, 0xFA), dpp16(10, 0x140, 0x6, 0xF, false)]); // row_mirror row_mask:0x6
            body.extend([vop1(7, V_MOV, 233), dpp8_reverse << 8 | 10]); // dpp8:[7,6,5,4,3,2,1,0]
            body.extend([sop1(22, S_MOV_B32, LIT), 0x7654_3210, sop1(23, S_MOV_B32, LIT), 0xFEDC_BA98]);
            body.extend(vop3(V_PERMLANEX16, 8, v(10), 22, 23));
            if crossing {
                body.extend(vop3(V_READLANE, 24, v(10), k(33), 0));
                body.push(vop1(12, V_MOV, 24));
            } else {
                body.push(vop1(12, V_MOV, k(0)));
            }
            for (row, reg) in [3u32, 4, 5, 6, 7, 8, 12].into_iter().enumerate() {
                body.extend(store(reg, 9, 4 * n * row as u32));
            }
            let case = Case {
                name: &format!("half-wave cross-lane ops, {n} threads{}", if crossing { ", exchanging" } else { "" }),
                body,
                threads: n,
                lds: 0,
                outs: 7,
                wide: false,
            };
            let o = check(&case, &[[3, 0], [1000, 0]], |[s0, _]| {
                let mut out = vec![0; 7 * n as usize];
                for (t, w, l) in threads(n) {
                    let at = |lane: u32| val(64 * w + lane, s0);
                    let rows = [
                        if l & 15 >= 1 { at(l - 1) } else { 0 },
                        at((l & 32) | ((l & 31) ^ 17)),
                        at((l & !3) | (3 - (l & 3))),
                        if matches!(l >> 4, 1 | 2) { at(l ^ 15) } else { 0 },
                        at((l & !7) | (7 - (l & 7))),
                        at(l ^ 16),
                        if crossing { at(33) } else { 0 },
                    ];
                    for (r, x) in rows.into_iter().enumerate() {
                        out[r * n as usize + t as usize] = x;
                    }
                }
                out
            });
            o.exact(&case.name);
        }
    }
}

/// LDS the console's wave shares without a barrier: each thread writes its dword, then reads
/// the dword of the thread 32 away (in the other half of its console wave).
#[test]
fn lds_shared_within_the_console_wave_without_a_barrier() {
    for n in [64u32, 128] {
        let mut body = Vec::new();
        body.extend(ds(DS_WRITE_B32, 0, 9, 10, 0)); // ds_write_b32 v9, v10
        body.extend([vop2(V_XOR, 4, LIT, 9), 0x80]); // v4 = v9 ^ 128: thread t ^ 32
        body.extend(ds(DS_READ_B32, 3, 4, 0, 0)); // ds_read_b32 v3, v4
        body.extend(store(3, 9, 0));
        let case = Case {
            name: &format!("LDS across the halves without a barrier, {n} threads"),
            body,
            threads: n,
            lds: 4 * n,
            outs: 1,
            wide: false,
        };
        let o = check(&case, &[[3, 0], [77, 0]], |[s0, _]| threads(n).map(|(t, _, _)| val(t ^ 32, s0)).collect());
        if n == 64 {
            o.exact(&case.name);
        } else {
            // A group of several console waves keeps no barrier in its code where HLSL needs
            // one (the console compiler drops `s_barrier` only for a group of one wave), so
            // the translation does not order LDS within a console wave there: documented.
            eprintln!("{}: {o:?}", case.name);
        }
    }
}

/// `ds_bpermute_b32` from the lane 63 - l (in the other half): exact or refused, never wrong.
#[test]
fn a_backward_permute_across_the_halves_is_exact_or_refused() {
    for n in [64u32, 128] {
        let mut body = vec![
            vop2(0x26, 4, LIT, 11), // v_sub_nc_u32 v4, 63, v11
            63,
            vop2(V_LSHLREV, 4, k(2), 4), // v4 = 4 (63 - l)
        ];
        body.extend(ds(DS_BPERMUTE, 3, 4, 10, 0)); // ds_bpermute_b32 v3, v4, v10
        body.extend(store(3, 9, 0));
        let case = Case {
            name: &format!("ds_bpermute across the halves, {n} threads"),
            body,
            threads: n,
            lds: 0,
            outs: 1,
            wide: false,
        };
        let o = check(&case, &[[3, 0]], |[s0, _]| threads(n).map(|(_, w, l)| val(64 * w + 63 - l, s0)).collect());
        eprintln!("{}: {o:?}", case.name);
        assert!(!matches!(o, Outcome::Differs(_)), "{}: {o:?}", case.name);
    }
}

/// `WaveActiveSum` as the console compiler expands it (a whole-wave section: DPP, permlanex16,
/// `v_readlane` 32 and 0) inside `if (lane < s1)`: the sum over the console wave's lanes in
/// EXEC, which the lanes inside store.
#[test]
fn a_wave_sum_inside_a_lane_branch() {
    let sum_section: Vec<u32> = super::wave::WAVE_OR_V7_TO_S7
        .iter()
        .map(|&w| if w >> 25 == 0x1C { w & 0x01FF_FFFF | V_ADD << 25 } else { w })
        .collect();
    for n in [64u32, 128] {
        let mut inner = sum_section.clone();
        inner.push(vop1(3, V_MOV, 7));
        inner.extend(store(3, 9, 0));
        let mut body = vec![
            vop1(7, V_MOV, v(10)),
            vopc(V_CMP_GT_U32, 1, 11),                 // v_cmp_gt_u32 vcc, s1, v11: lanes below s1
            sop1(12, S_AND_SAVEEXEC_B64, VCC),         // s_and_saveexec_b64 s[12:13], vcc
            sopp(S_CBRANCH_EXECZ, inner.len() as u32), // past the body
        ];
        body.extend(inner);
        body.push(sop1(EXEC, S_MOV_B64, 12));
        for wide in [false, true] {
            let case = Case {
                name: &format!("wave sum in a lane branch, {n} threads{}", if wide { ", wave-wide" } else { "" }),
                body: body.clone(),
                threads: n,
                lds: 0,
                outs: 1,
                wide,
            };
            let model = |[s0, s1]: [u32; 2]| -> Vec<u32> {
                threads(n)
                    .map(|(_, w, l)| {
                        if l < s1 {
                            (0..s1.min(64)).map(|j| val(64 * w + j, s0)).fold(0u32, u32::wrapping_add)
                        } else {
                            0
                        }
                    })
                    .collect()
            };
            let consts = [[3, 64], [3, 40], [3, 33], [3, 32], [3, 10], [3, 1]];
            check(&case, &consts, model).exact(&case.name);
        }
    }
}

/// The DPP row fix (`VICTORY_WAVE_DPP_ROWS`, on by default) changes a translation only where a
/// row mask's bits for rows 2-3 differ from those for rows 0-1, in a console wave64 program run
/// as host waves of 32: every other program here translates byte for byte as with it off.
#[test]
fn the_dpp_row_fix_changes_only_the_programs_it_is_for() {
    let translated = |body: &[u32], threads: u32, on: bool| {
        let was = lower::TEST_DPP_ROWS.replace(Some(on));
        let r = translate(&native(body, threads, 0), false).map(|s| s.dxil);
        lower::TEST_DPP_ROWS.set(was);
        r.map_err(|e| e.to_string())
    };
    let mut changed = Vec::new();
    for crossing in [false, true] {
        for mask in [0xFu32, 0x5, 0xA, 0x6, 0x3, 0x9, 0xC] {
            let mut body = vec![vop1(6, V_MOV, k(0))];
            body.extend([vop1(6, V_MOV, 0xFA), dpp16(10, 0x140, mask, 0xF, false)]); // row_mirror
            body.extend([vop1(3, V_MOV, 0xFA), dpp16(10, 0x111, mask, 0x5, false)]); // row_shr:1
            if crossing {
                body.extend(vop3(V_READLANE, 24, v(10), k(33), 0));
                body.push(vop1(12, V_MOV, 24));
                body.extend(store(12, 9, 512));
            }
            body.extend(store(6, 9, 0));
            body.extend(store(3, 9, 256));
            for n in [64u32, 128] {
                let (off, on) = (translated(&body, n, false), translated(&body, n, true));
                assert!(off.is_ok() && on.is_ok(), "{off:?} {on:?}");
                let differs = mask >> 2 & 3 != mask & 3;
                assert_eq!(off != on, differs, "row_mask {mask:#x}, {n} threads, crossing {crossing}");
                if differs {
                    changed.push((mask, n, crossing));
                }
            }
        }
    }
    // (the programs without DPP row masks at all)
    let mut first = vec![vopc(V_CMPX_LE_U32, 1, 11), vop1(20, V_READFIRSTLANE, v(10)), vop1(3, V_MOV, 20)];
    first.extend(store(3, 9, 0));
    let mut lanes = Vec::new();
    lanes.extend(vop3(V_READLANE, 20, v(10), k(45), 0));
    lanes.push(vop1(3, V_MOV, 20));
    lanes.extend(store(3, 9, 0));
    for body in [first, lanes] {
        for n in [64u32, 128] {
            assert_eq!(translated(&body, n, false), translated(&body, n, true));
        }
    }
    eprintln!("changed by the DPP row fix: {changed:?}");
}
/// `VICTORY_WAVE_LDS_OWN=1` is `emit`'s last form, for programs every other form leaves with
/// 64-lane waves: a program that translates without it translates byte for byte the same with it.
#[test]
fn the_lds_own_form_changes_no_program_that_translates_without_it() {
    let translated = |body: &[u32], threads: u32, lds: u32, on: bool| {
        let was = lower::TEST_LDS_OWN.replace(Some(on));
        let r = translate(&native(body, threads, lds), false).map(|s| s.dxil);
        lower::TEST_LDS_OWN.set(was);
        r.map_err(|e| e.to_string())
    };
    let mut shared = Vec::new();
    shared.extend(ds(DS_WRITE_B32, 0, 9, 10, 0));
    shared.extend([vop2(V_XOR, 4, LIT, 9), 0x80]);
    shared.extend(ds(DS_READ_B32, 3, 4, 0, 0));
    shared.extend(store(3, 9, 0));
    let mut first = vec![vopc(V_CMPX_LE_U32, 1, 11), vop1(20, V_READFIRSTLANE, v(10)), vop1(3, V_MOV, 20)];
    first.extend(store(3, 9, 0));
    let mut lanes = Vec::new();
    lanes.extend(vop3(V_READLANE, 20, v(10), k(45), 0));
    lanes.push(vop1(3, V_MOV, 20));
    lanes.extend(store(3, 9, 0));
    let mut both = lanes.clone();
    both.extend(shared.clone());
    for (body, per_thread_lds) in [(shared, 4), (first, 0), (lanes, 0), (both, 4)] {
        for n in [64u32, 128] {
            let off = translated(&body, n, per_thread_lds * n, false);
            let on = translated(&body, n, per_thread_lds * n, true);
            assert_eq!(off, on, "{n} threads");
        }
    }
}
