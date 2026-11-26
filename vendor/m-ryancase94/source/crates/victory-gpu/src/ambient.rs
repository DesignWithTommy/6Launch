//! A stand-in for the title's ray-traced indirect diffuse light, on unless
//! `VICTORY_AMBIENT=off`.
//!
//! Ported from Belobug/VIctory-Belobug-AMD (beloooo: 0603123, 4ffc476,
//! cd9f78c, 4f45ab8; GPL-3.0-or-later) through uiliannn/VItoriaBr's port
//! (d0af132, uiliannn), with these changes: three ways to apply it, the
//! views it hooks settable, and a line in the log for each reason it does
//! nothing. (Off by default there; on at 0.6 here, as this fork's earlier
//! port of Belobug's pass was.)
//!
//! The title's indirect diffuse light comes from screen probes it fills by
//! tracing rays through ray-tracing structures, which the structure builder
//! does not build here (`VICTORY_BVH_BUILDER` off). Its program that resolves
//! the probes (`CS_ScreenProbes_TemporalResolve_*`) writes the light as
//! R9G9B9E5 into an image; right after it, this pass changes that image. The
//! title's own lighting then applies it with the surfaces' colours. The
//! resolve's output is also the next frame's history, so every way here is
//! stable from frame to frame (no multiplying).
//!
//! - `VICTORY_AMBIENT=<level>` sets it (unset: 0.6; `off`: the title's own
//!   output, untouched; 0: no indirect diffuse light at all). The level is in
//!   the light's own units: about 0.6 is what Belobug found balanced indoors.
//! - `VICTORY_AMBIENT_MODE`:
//!   - `replace` (the default, as Belobug's): the output is the level, tinted
//!     and shaded. For runs whose tracers do not run or read garbage.
//!   - `floor`: the title's output, raised to at least that (per channel).
//!     Keeps the title's sky light outdoors; lifts black interiors and faces.
//!   - `cap`: the title's output, its brightest channel held to at most the
//!     level times the shading, colour kept. Holds back sky light that leaks
//!     into rooms when every ray misses (`VICTORY_BVH_MISS=1`).
//! - `VICTORY_AMBIENT_TINT=r,g,b`: the colour (warm white by default; not
//!   used by `cap`).
//! - `VICTORY_AMBIENT_SHADE=s,r,n,t`: shading in creases and under objects
//!   from the scene's depth: strength (0 for none), radius in metres, near
//!   plane, tangent of half the vertical field of view.
//! - `VICTORY_AMBIENT_VIEWS=ls,lo,ds,do`: where the resolve's light image
//!   (UAV) and depth (SRV) are: root parameter and dword in its table
//!   (default `1,176,0,464`, Belobug's). A diagnostics build lists what the
//!   resolve reads ("the light resolve reads") to find others.
//! - `VICTORY_AMBIENT_FILE=<file>` (diagnostics builds): `level r g b` and
//!   optionally `s r n t`, read again twice a second, to tune by eye.
//!
//! shortcut: the same light from every direction; gone once rays run.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::s;

use crate::host::{HostList, host};
use crate::object::This;

/// The programs it follows, by the start of their name.
pub(crate) const PROGRAM: &[u8] = b"CS_ScreenProbes_TemporalResolve";

/// Logs `$($t)*` the first time this line is reached.
macro_rules! once_log {
    ($level:ident, $($t:tt)*) => {{
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::$level!($($t)*));
    }};
}

/// Where the resolve writes the indirect diffuse light (R9G9B9E5 as 32-bit
/// integers) and reads the scene's depth (reversed, D32 with stencil): root
/// parameter and dword in its table, each. Belobug's, or
/// `VICTORY_AMBIENT_VIEWS`.
pub(crate) fn views() -> [(u8, u32); 2] {
    static VIEWS: OnceLock<[(u8, u32); 2]> = OnceLock::new();
    *VIEWS.get_or_init(|| {
        let set: Vec<u32> = std::env::var("VICTORY_AMBIENT_VIEWS")
            .unwrap_or_default()
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        match set[..] {
            [ls, lo, ds, d_o] if ls < 256 && ds < 256 => [(ls as u8, lo), (ds as u8, d_o)],
            _ => [(1, 176), (0, 464)],
        }
    })
}

/// How the level is applied (`VICTORY_AMBIENT_MODE`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Replace = 0,
    Floor = 1,
    Cap = 2,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_AMBIENT_MODE").ok().as_deref().map(str::trim) {
        Some("floor") => Mode::Floor,
        Some("cap") => Mode::Cap,
        _ => Mode::Replace,
    })
}

/// The level when `VICTORY_AMBIENT` gives none: 0.6 looked balanced indoors
/// (Belobug's finding; run a2 here).
const DEFAULT: f32 = 0.6;

/// The level; `None` with `VICTORY_AMBIENT=off`.
pub(crate) fn level() -> Option<f32> {
    static LEVEL: OnceLock<Option<f32>> = OnceLock::new();
    *LEVEL.get_or_init(|| parse_level(std::env::var("VICTORY_AMBIENT").ok().as_deref()))
}

/// `VICTORY_AMBIENT`'s meaning: the default when it is unset or not a level.
fn parse_level(set: Option<&str>) -> Option<f32> {
    let Some(set) = set.map(str::trim) else {
        return Some(DEFAULT);
    };
    if set == "off" {
        return None;
    }
    let level = set.parse().ok().filter(|l: &f32| l.is_finite() && *l >= 0.0);
    if level.is_none() {
        tracing::warn!(value = set, default = DEFAULT, "VICTORY_AMBIENT is not a level: the default is used");
    }
    Some(level.unwrap_or(DEFAULT))
}

/// The light's colour: warm white, or what `VICTORY_AMBIENT_TINT=r,g,b`
/// gives, or the file's (`VICTORY_AMBIENT_FILE`).
fn tint() -> [f32; 3] {
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, tint)) = *live {
        return tint;
    }
    static SET: OnceLock<[f32; 3]> = OnceLock::new();
    *SET.get_or_init(|| {
        let set: Vec<f32> = std::env::var("VICTORY_AMBIENT_TINT")
            .unwrap_or_default()
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        match set[..] {
            [r, g, b] => [r, g, b],
            _ => [1.0, 0.98, 0.94],
        }
    })
}

/// The level and colour last read from `VICTORY_AMBIENT_FILE`, if any.
static LIVE: std::sync::Mutex<Option<(f32, [f32; 3])>> = std::sync::Mutex::new(None);

/// How much less of the light reaches creases and corners, worked out from
/// the depth image alone: strength (0 for none), the radius looked at in
/// metres, the near plane's distance, and the tangent of half the vertical
/// field of view (the last two are guesses the picture is not sensitive
/// to). `VICTORY_AMBIENT_SHADE=s,r,n,t` sets them; the file's values after
/// its first four replace them.
/// shortcut: stands in for the occlusion the title's traced probes carry.
static SHADE: std::sync::Mutex<Option<[f32; 4]>> = std::sync::Mutex::new(None);

fn shade() -> [f32; 4] {
    if let Some(live) = *SHADE.lock().unwrap_or_else(|e| e.into_inner()) {
        return live;
    }
    static SET: OnceLock<[f32; 4]> = OnceLock::new();
    *SET.get_or_init(|| {
        let mut shade = [1.0, 0.6, 0.1, 0.45];
        let set = std::env::var("VICTORY_AMBIENT_SHADE").unwrap_or_default();
        for (to, v) in shade.iter_mut().zip(set.split(',')) {
            if let Ok(v) = v.trim().parse() {
                *to = v;
            }
        }
        shade
    })
}

/// The level from `VICTORY_AMBIENT_FILE`, if it gave one; starts the thread
/// that reads the file (diagnostics builds).
fn live_level() -> Option<f32> {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        if !cfg!(feature = "diagnostics") {
            return;
        }
        let Some(file) = std::env::var_os("VICTORY_AMBIENT_FILE") else {
            return;
        };
        let _ = std::thread::Builder::new().name("victory-ambient".into()).spawn(move || {
            loop {
                let read: Vec<f32> = std::fs::read_to_string(&file)
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter_map(|v| v.parse().ok())
                    .collect();
                if let [level, r, g, b, ref rest @ ..] = read[..] {
                    if let [s, rad, n, t] = rest[..] {
                        *SHADE.lock().unwrap_or_else(|e| e.into_inner()) = Some([s, rad, n, t]);
                    }
                    let new = Some((level.max(0.0), [r, g, b]));
                    let mut live = LIVE.lock().unwrap_or_else(|e| e.into_inner());
                    if *live != new {
                        tracing::info!(level, r, g, b, "ambient light set from the file");
                        *live = new;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
    });
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).map(|(level, _)| level)
}

const SOURCE: &str = r#"
#define RS "RootConstants(num32BitConstants=9, b0), DescriptorTable(UAV(u0), SRV(t0))"
cbuffer Floor : register(b0) {
    float level; float3 tint;
    float strength; float radius; float near; float tan_y;
    uint mode;
};
RWTexture2D<uint> light : register(u0);
Texture2D<float> depth : register(t0);
// The point seen at a texel, in metres from the eye (x right, y up, z away).
float3 seen(int2 p, float2 size) {
    float d = depth.Load(int3(clamp(p, int2(0, 0), int2(size) - 1), 0));
    float z = near / max(d, 1e-7);
    float2 ndc = (float2(p) + 0.5) / size * 2.0 - 1.0;
    return float3(ndc.x * tan_y * size.x / size.y * z, -ndc.y * tan_y * z, z);
}
// How much of the light from around reaches the texel: 1 in the open.
float open_to_light(int2 p, float2 size) {
    float3 c = seen(p, size);
    if (c.z > 400.0) return 1.0;
    float3 l = seen(p + int2(-1, 0), size), r = seen(p + int2(1, 0), size);
    float3 u = seen(p + int2(0, -1), size), d = seen(p + int2(0, 1), size);
    float3 dx = abs(l.z - c.z) < abs(r.z - c.z) ? c - l : r - c;
    float3 dy = abs(u.z - c.z) < abs(d.z - c.z) ? c - u : d - c;
    float3 n = normalize(cross(dy, dx));
    if (dot(n, c) > 0.0) n = -n;
    float px = radius * size.y / (2.0 * tan_y * c.z);
    if (px < 1.5) return 1.0;
    px = min(px, 128.0);
    float noise = frac(52.9829189 * frac(dot(float2(p), float2(0.06711056, 0.00583715))));
    float sum = 0.0;
    const int N = 16;
    for (int i = 0; i < N; i++) {
        float a = i * 2.399963 + noise * 6.2831853;
        float2 o = float2(cos(a), sin(a)) * (sqrt((i + 0.5) / N) * px);
        float3 v = seen(p + int2(round(o)), size) - c;
        float vv = dot(v, v);
        float over = dot(v, n) - 0.004 * c.z - 0.005;
        sum += max(over, 0.0) / sqrt(vv + 1e-6) * saturate(1.0 - vv / (radius * radius));
    }
    return saturate(1.0 - strength * 2.0 * sum / N);
}
float3 unpack(uint v) {
    return float3(v & 511, (v >> 9) & 511, (v >> 18) & 511) * exp2(float(v >> 27) - 24.0);
}
uint pack(float3 c) {
    c = clamp(c, 0.0, 65408.0);
    float most = max(c.r, max(c.g, c.b));
    int e = clamp(int(floor(log2(max(most, 1e-9)))) + 16, 0, 31);
    float scale = exp2(float(24 - e));
    if (most * scale >= 511.5) { e = min(e + 1, 31); scale = exp2(float(24 - e)); }
    uint3 m = min(uint3(c * scale + 0.5), 511u);
    return m.r | (m.g << 9) | (m.b << 18) | (uint(e) << 27);
}
[RootSignature(RS)]
[numthreads(8, 8, 1)]
void main(uint3 at : SV_DispatchThreadID) {
    uint w, h;
    light.GetDimensions(w, h);
    if (at.x >= w || at.y >= h) return;
    float open = strength > 0.0 ? open_to_light(int2(at.xy), float2(w, h)) : 1.0;
    float3 target = tint * level * open;
    if (mode == 0) {
        // What the resolve left is not kept: without rays its inputs may
        // never be written and hold whatever their memory did (Belobug run
        // v6: bright magenta on translucent surfaces, blocks of colours).
        light[at.xy] = pack(target);
        return;
    }
    float3 was = unpack(light[at.xy]);
    if (mode == 1) {
        light[at.xy] = pack(max(was, target));
    } else {
        float most = max(was.r, max(was.g, was.b));
        float most_allowed = level * open;
        light[at.xy] = pack(most > most_allowed ? was * (most_allowed / most) : was);
    }
}
"#;

/// Views, used in turn: more than the passes of every list in flight.
const SLOTS: u32 = 1024;

struct Pass {
    root: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    views: ID3D12DescriptorHeap,
    step: u32,
    next: AtomicU32,
}
// SAFETY: D3D12 objects are free-threaded.
unsafe impl Send for Pass {}
// SAFETY: as above.
unsafe impl Sync for Pass {}

fn make(device: &ID3D12Device) -> windows::core::Result<Pass> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source and names live through the call; out pointers ours.
    let compiled = unsafe {
        D3DCompile(
            SOURCE.as_ptr().cast(),
            SOURCE.len(),
            s!("ambient"),
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
    let text = |b: &ID3DBlob| {
        // SAFETY: the blob's own bytes.
        unsafe { std::slice::from_raw_parts(b.GetBufferPointer().cast::<u8>(), b.GetBufferSize()) }
            .to_vec()
    };
    if let Err(e) = compiled {
        if let Some(errors) = errors {
            tracing::warn!("ambient light: {}", String::from_utf8_lossy(&text(&errors)));
        }
        return Err(e);
    }
    let code = text(&code.ok_or_else(windows::core::Error::empty)?);
    // SAFETY: the root signature is inside the shader; plain creations.
    unsafe {
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &code)?;
        let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: code.as_ptr().cast(),
                BytecodeLength: code.len(),
            },
            ..Default::default()
        })?;
        let kind = D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV;
        let views = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: kind,
            NumDescriptors: SLOTS * 2,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;
        Ok(Pass {
            root,
            pso,
            views,
            step: device.GetDescriptorHandleIncrementSize(kind),
            next: AtomicU32::new(0),
        })
    }
}

fn uav_barrier(list: &ID3D12GraphicsCommandList) {
    let barrier = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_UAV,
        ..Default::default()
    };
    // SAFETY: an open list; the barrier names no resource.
    unsafe { list.ResourceBarrier(&[barrier]) };
}

/// Changes what `light` holds, in `list`, after the program that wrote it as
/// an image. Whether it recorded anything (the list's heaps, compute root
/// signature and pipeline are then ours: the caller has the title's set
/// again).
pub(crate) fn raise(list: &HostList, light: This, depth: Option<This>) -> bool {
    let (Some(level), Some(host)) = (level(), host()) else {
        return false;
    };
    let Some(light) = crate::resource::host_made(light) else {
        once_log!(warn, "ambient stand-in: the resolve's light image has no host copy yet (waiting)");
        return false;
    };
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    let Some(pass) = PASS
        .get_or_init(|| {
            make(&host.device)
                .inspect_err(|e| tracing::warn!(error = %e, "ambient stand-in: its pass could not be made; off"))
                .ok()
        })
        .as_ref()
    else {
        return false;
    };
    // SAFETY: a live resource.
    let d = unsafe { light.GetDesc() };
    if d.Format != DXGI_FORMAT_R32_UINT {
        once_log!(
            warn,
            format = d.Format.0,
            width = d.Width,
            height = d.Height,
            "ambient stand-in off: the resolve's light image is not R32_UINT (R9G9B9E5 bits); try other VICTORY_AMBIENT_VIEWS"
        );
        return false;
    }
    // The depth the resolve just read, in the state it read it in; of the
    // light's size, or the shading is left out.
    let depth_found = depth.and_then(crate::resource::host_made);
    let depth = depth_found.filter(|t| {
        // SAFETY: a live resource.
        let t = unsafe { t.GetDesc() };
        t.Format == DXGI_FORMAT_R32G8X24_TYPELESS && (t.Width, t.Height) == (d.Width, d.Height)
    });
    if depth.is_none() {
        match depth_found {
            Some(t) => {
                // SAFETY: a live resource.
                let t = unsafe { t.GetDesc() };
                once_log!(
                    warn,
                    format = t.Format.0,
                    width = t.Width,
                    height = t.Height,
                    light_width = d.Width,
                    light_height = d.Height,
                    "ambient stand-in: the depth found is not D32S8 of the light's size; no shading in creases"
                );
            }
            None => once_log!(warn, "ambient stand-in: no depth at the depth view; no shading in creases"),
        }
    }
    once_log!(
        info,
        level,
        mode = ?mode(),
        width = d.Width,
        height = d.Height,
        shaded = depth.is_some(),
        "ambient stand-in on: the indirect-light resolve's output is changed after it runs"
    );
    let slot = pass.next.fetch_add(1, Ordering::Relaxed) % SLOTS * 2;
    list.keep(light);
    if let Some(depth) = depth {
        list.keep(depth);
    }
    let l = &list.list;
    uav_barrier(l);
    // SAFETY: an open list; a live texture the list keeps, in the
    // unordered-access state as the program before left it; a view in a slot
    // no list in flight reads.
    unsafe {
        let cpu = pass.views.GetCPUDescriptorHandleForHeapStart();
        let gpu = pass.views.GetGPUDescriptorHandleForHeapStart();
        let at = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: cpu.ptr + (slot * pass.step) as usize,
        };
        let view = D3D12_UNORDERED_ACCESS_VIEW_DESC {
            Format: DXGI_FORMAT_R32_UINT,
            ViewDimension: D3D12_UAV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        host.device.CreateUnorderedAccessView(light, None::<&ID3D12Resource>, Some(&view), at);
        let seen = D3D12_SHADER_RESOURCE_VIEW_DESC {
            Format: DXGI_FORMAT_R32_FLOAT_X8X24_TYPELESS,
            ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
            Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
            Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() },
            },
        };
        let beside = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: at.ptr + pass.step as usize };
        host.device.CreateShaderResourceView(depth, Some(&seen), beside);
        l.SetDescriptorHeaps(&[Some(pass.views.clone())]);
        l.SetComputeRootSignature(&pass.root);
        l.SetPipelineState(&pass.pso);
        let [r, g, b] = tint();
        let level = live_level().unwrap_or(level);
        let [strength, radius, near, tan_y] = shade();
        let strength = if depth.is_some() { strength } else { 0.0 };
        let constants: [u32; 9] = [
            level.to_bits(),
            r.to_bits(),
            g.to_bits(),
            b.to_bits(),
            strength.to_bits(),
            radius.to_bits(),
            near.to_bits(),
            tan_y.to_bits(),
            mode() as u32,
        ];
        l.SetComputeRoot32BitConstants(0, 9, constants.as_ptr().cast(), 0);
        l.SetComputeRootDescriptorTable(
            1,
            D3D12_GPU_DESCRIPTOR_HANDLE {
                ptr: gpu.ptr + u64::from(slot * pass.step),
            },
        );
        l.Dispatch((d.Width as u32).div_ceil(8), d.Height.div_ceil(8), 1);
    }
    uav_barrier(l);
    true
}

#[cfg(test)]
mod tests {
    /// Unset is the default level, `off` is off, a number is that level.
    #[test]
    fn the_switch_is_read_as_before() {
        use super::{DEFAULT, parse_level};
        assert_eq!(parse_level(None), Some(DEFAULT));
        assert_eq!(parse_level(Some("off")), None);
        assert_eq!(parse_level(Some(" 0.25 ")), Some(0.25));
        assert_eq!(parse_level(Some("0")), Some(0.0));
        assert_eq!(parse_level(Some("-1")), Some(DEFAULT));
        assert_eq!(parse_level(Some("bright")), Some(DEFAULT));
    }

    /// The pass compiles with the system's compiler and the host takes it.
    #[test]
    fn ambient_pass_builds() {
        let Some(host) = crate::host::host() else {
            return;
        };
        super::make(&host.device).expect("the ambient pass");
    }
}
