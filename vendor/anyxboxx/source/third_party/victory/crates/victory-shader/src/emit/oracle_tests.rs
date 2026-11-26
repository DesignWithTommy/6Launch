//! Differential runs of whole console compute programs against a reference
//! interpreter (AnyXboxX's RDNA 2 interpreter, run outside this crate).
//!
//! Nothing here holds or embeds title code: the programs are read at test
//! time from containers the user dumped from their own copy of the title.
//! Without `VICTORY_ORACLE_DIR` both tests return at once.
//!
//! * `oracle_dump`: for each `*.cdxil` in `VICTORY_ORACLE_CONTAINERS`,
//!   writes `<stem>.meta` (native program layout and what the translation
//!   binds) and `<stem>.code` (the GFX10 code) to `VICTORY_ORACLE_DIR`.
//! * `oracle_run`: for each `<stem>.plan` there (written by the
//!   interpreter's side: views, user data, group counts), translates the
//!   container, dispatches it and writes every UAV's words in heap order to
//!   `<stem>.gpu` (`<stem>.warp` with `VICTORY_WARP=1`).
//!
//! Plan lines: `container <path>`, `groups <x> <y> <z>`,
//! `sgpr <user-data index> <value>`, `table <user-data index> <heap slot>`,
//! and one `view` line per heap slot in order:
//! `view typed <dxgi format> <element bytes> <stride word> <hex dwords...>`,
//! `view raw <hex dwords...>`, `view cbv <hex dwords...>`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_R32_TYPELESS};

use super::host::{Dispatch, Host, Root, Stride, View};
use super::valu_tests::ONE_AT_A_TIME;
use super::{SLOT_STRIDES, SLOT_TABLE_STRIDES, translate};
use crate::dxbc::{Container, FourCC};
use crate::native::{HwStage, RootType, StageIo};

fn dirs() -> Option<(PathBuf, Option<PathBuf>)> {
    let out = PathBuf::from(std::env::var_os("VICTORY_ORACLE_DIR")?);
    let containers = std::env::var_os("VICTORY_ORACLE_CONTAINERS").map(PathBuf::from);
    Some((out, containers))
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

fn meta(container: &[u8]) -> Result<(String, Vec<u8>), String> {
    let c = Container::parse(container).map_err(|e| e.to_string())?;
    let dxil = c
        .part(FourCC::DXIL)
        .or(c.part(FourCC::ACCC))
        .ok_or("no DXIL part")?;
    let native = crate::native_programs(dxil.data)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.stage == HwStage::Cs)
        .ok_or("no compute program")?;
    let StageIo::Cs(cs) = &native.io else {
        return Err("no compute io".into());
    };
    let mut m = String::new();
    writeln!(m, "wave64 {}", u8::from(native.wave64)).unwrap();
    writeln!(m, "vgprs {}", native.vgprs).unwrap();
    writeln!(m, "rsrc2 {:#x}", cs.rsrc2).unwrap();
    let [x, y, z] = cs.threads;
    writeln!(m, "threads {x} {y} {z}").unwrap();
    for p in &native.layout {
        writeln!(
            m,
            "param {:?} {} {} {} {} {}",
            p.root, p.table_offset_dwords, p.sgprs, p.base_register, p.user_data, p.slot
        )
        .unwrap();
    }
    for u in &native.used {
        writeln!(
            m,
            "used {:?} {:?} {} {} {} {} {} {}",
            u.kind,
            u.root,
            u.table_offset_dwords,
            u.lower_bound,
            u.stride,
            u.dwords,
            u.user_data,
            u.slot
        )
        .unwrap();
    }
    match translate(container) {
        Ok(s) => {
            writeln!(m, "translate ok").unwrap();
            for u in &s.user_data {
                writeln!(
                    m,
                    "user_data {} {} {:?} {}",
                    u.index, u.slot, u.root, u.dword
                )
                .unwrap();
            }
            for r in &s.root_cbvs {
                writeln!(m, "root_cbv {r}").unwrap();
            }
            for r in &s.root_srvs {
                writeln!(m, "root_srv {r}").unwrap();
            }
            for r in &s.root_uavs {
                writeln!(m, "root_uav {r}").unwrap();
            }
        }
        Err(e) => writeln!(m, "translate err {e}").unwrap(),
    }
    Ok((m, native.code.clone()))
}

#[test]
fn oracle_dump() {
    let Some((out, Some(containers))) = dirs() else {
        eprintln!("VICTORY_ORACLE_DIR / VICTORY_ORACLE_CONTAINERS unset; skipped");
        return;
    };
    std::fs::create_dir_all(&out).unwrap();
    let mut paths: Vec<_> = std::fs::read_dir(&containers)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "cdxil"))
        .collect();
    paths.sort();
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let s = stem(&path);
        match meta(&bytes) {
            Ok((m, code)) => {
                let m = format!("container {}\n{m}", path.display());
                std::fs::write(out.join(format!("{s}.meta")), m).unwrap();
                std::fs::write(out.join(format!("{s}.code")), code).unwrap();
            }
            Err(e) => eprintln!("{s}: {e}"),
        }
    }
}

enum PlanView {
    /// `srv`: a shader resource view (no output), else an unordered one.
    Typed {
        format: u32,
        bytes: u32,
        stride: u32,
        data: Vec<u32>,
        srv: bool,
    },
    Raw(Vec<u32>, bool),
    /// A UAV over the buffer of the UAV at heap slot `of` (same title memory).
    Alias {
        of: usize,
        format: u32,
        bytes: u32,
        stride: u32,
    },
    Cbv(Vec<u32>),
}

struct Plan {
    container: PathBuf,
    /// Replacement GFX10 code (a cut or instrumented copy, for bisecting).
    code: Option<(u32, PathBuf)>,
    groups: [u32; 3],
    sgprs: Vec<(u32, u32)>,
    tables: Vec<(u32, u32)>,
    views: Vec<PlanView>,
    /// Title addresses of the views by heap slot (`address <slot> <hex>`), for
    /// programs that read a view's address (`USER_VIEW_ADDRESS`).
    addresses: Vec<(u32, u64)>,
}

fn hex(words: &[&str]) -> Vec<u32> {
    words
        .iter()
        .map(|w| u32::from_str_radix(w, 16).unwrap())
        .collect()
}

fn parse_plan(text: &str) -> Plan {
    let mut plan = Plan {
        container: PathBuf::new(),
        code: None,
        groups: [1, 1, 1],
        sgprs: Vec::new(),
        tables: Vec::new(),
        views: Vec::new(),
        addresses: Vec::new(),
    };
    for line in text.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let n = |i: usize| w[i].parse::<u32>().unwrap();
        match w.first().copied() {
            Some("container") => plan.container = PathBuf::from(line[10..].trim()),
            Some("code") => {
                let rest = line[5..].trim();
                let (slot, path) = rest.split_once(' ').unwrap();
                plan.code = Some((slot.parse().unwrap(), PathBuf::from(path)));
            }
            Some("groups") => plan.groups = [n(1), n(2), n(3)],
            Some("sgpr") => plan.sgprs.push((n(1), n(2))),
            Some("table") => plan.tables.push((n(1), n(2))),
            Some("address") => plan.addresses.push((n(1), u64::from_str_radix(w[2], 16).unwrap())),
            Some("view") => plan.views.push(match w[1] {
                kind @ ("typed" | "typedsrv") => PlanView::Typed {
                    format: n(2),
                    bytes: n(3),
                    stride: n(4),
                    data: hex(&w[5..]),
                    srv: kind == "typedsrv",
                },
                "raw" => PlanView::Raw(hex(&w[2..]), false),
                "rawsrv" => PlanView::Raw(hex(&w[2..]), true),
                "alias" => PlanView::Alias {
                    of: n(2) as usize,
                    format: n(3),
                    bytes: n(4),
                    stride: n(5),
                },
                "cbv" => PlanView::Cbv(hex(&w[2..])),
                other => panic!("unknown view {other}"),
            }),
            _ => {}
        }
    }
    plan
}

/// [`translate`] of the container's compute program with its code replaced,
/// and a raw UAV more in its first table at `debug_slot`.
fn translate_with_code(
    container: &[u8],
    code: Vec<u8>,
    debug_slot: u32,
) -> Result<super::HostShader, crate::ShaderError> {
    let c = Container::parse(container)?;
    let dxil = c
        .part(FourCC::DXIL)
        .ok_or(crate::ShaderError::Malformed("no DXIL part"))?;
    let mut native = crate::native_programs(dxil.data)?
        .into_iter()
        .find(|p| p.stage == HwStage::Cs)
        .ok_or(crate::ShaderError::Malformed("no compute program"))?;
    native.code = code;
    // (the cut keeps the thread index in v255)
    native.vgprs = 256;
    if let Some(t) = native.layout.iter().find(|p| p.root == RootType::Table) {
        native.used.push(crate::native::UsedResource {
            kind: crate::native::ResourceType::Uav,
            root: RootType::Table,
            table_offset_dwords: debug_slot * 8,
            lower_bound: 99,
            stride: 4,
            dwords: 4,
            user_data: t.user_data,
            slot: t.slot,
        });
    }
    let p = crate::spill::build(&native)?;
    let st = crate::structure::structure(&p);
    let res = crate::resource::track(&p, &native.layout, native.stage, st.as_ref().ok())?;
    let io = crate::io::raise(&p, &native)?;
    let st = st?;
    super::emit_with_retries(&c, &native, &p, &st, &io, &res)
}

/// Runs one plan; the UAV words in heap order.
fn run_plan(host: &Host, plan: &Plan) -> Result<Vec<Vec<u32>>, String> {
    let bytes = std::fs::read(&plan.container).map_err(|e| e.to_string())?;
    let shader = match &plan.code {
        None => translate(&bytes).map_err(|e| e.to_string())?,
        Some((slot, code)) => {
            let code = std::fs::read(code).map_err(|e| e.to_string())?;
            translate_with_code(&bytes, code, *slot).map_err(|e| e.to_string())?
        }
    };
    let (root, pso) = host.compute_pipeline(&shader)?;
    let mut ud = vec![0u32; shader.user_data_dwords().max(1) as usize];
    for u in &shader.user_data {
        let value = match u.root {
            RootType::Table => plan
                .tables
                .iter()
                .find(|t| t.0 == u.index)
                .map(|t| t.1 * 32),
            RootType::Constants => plan.sgprs.iter().find(|s| s.0 == u.index).map(|s| s.1),
            RootType::Other(kind)
                if kind == super::USER_VIEW_ADDRESS || kind == super::USER_VIEW_ADDRESS + 1 =>
            {
                let base = plan.tables.first().map_or(0, |t| t.1);
                plan.addresses
                    .iter()
                    .find(|a| a.0 == base + u.dword / 8)
                    .map(|a| if kind == super::USER_VIEW_ADDRESS { a.1 as u32 } else { (a.1 >> 32) as u32 })
            }
            _ => None,
        };
        ud[u.index as usize] = value.ok_or_else(|| format!("no plan value for user data {u:?}"))?;
    }
    let mut strides: Vec<u32> = plan
        .views
        .iter()
        .map(|v| match v {
            PlanView::Typed { stride, .. } => *stride,
            PlanView::Raw(..) => 1,
            PlanView::Alias { stride, .. } => *stride,
            PlanView::Cbv(_) => 0,
        })
        .collect();
    strides.resize(strides.len().max(256), 0);
    let mut roots = vec![(0, Root::Constants(ud))];
    let mut root_views: Vec<(u8, bool)> = shader.root_cbvs.iter().map(|&s| (s, false)).collect();
    root_views.extend(shader.root_srvs.iter().map(|&s| (s, true)));
    root_views.sort_unstable();
    if !shader.root_uavs.is_empty() {
        return Err(format!("root UAVs {:?}", shader.root_uavs));
    }
    for (n, (slot, srv)) in root_views.into_iter().enumerate() {
        if !(srv && (slot == SLOT_STRIDES || slot == SLOT_TABLE_STRIDES)) {
            return Err(format!("root view {slot} srv {srv}"));
        }
        roots.push((
            1 + n as u32,
            Root::View {
                srv,
                data: strides.clone(),
            },
        ));
    }
    let views: Vec<View> = plan
        .views
        .iter()
        .map(|v| match v {
            PlanView::Typed {
                format,
                bytes,
                data,
                srv: true,
                ..
            } => View::Buffer {
                data,
                stride: Stride::Format(DXGI_FORMAT(*format as i32), *bytes),
            },
            PlanView::Typed {
                format,
                bytes,
                data,
                ..
            } => View::TypedUav {
                data,
                format: DXGI_FORMAT(*format as i32),
                elem_bytes: *bytes,
            },
            PlanView::Raw(data, true) => View::Buffer {
                data,
                stride: Stride::Raw,
            },
            PlanView::Raw(data, false) => View::TypedUav {
                data,
                format: DXGI_FORMAT_R32_TYPELESS,
                elem_bytes: 4,
            },
            PlanView::Alias {
                of,
                format,
                bytes,
                ..
            } => View::AliasUav {
                of: *of,
                format: DXGI_FORMAT(*format as i32),
                elem_bytes: *bytes,
            },
            PlanView::Cbv(data) => View::Cbv(data),
        })
        .collect();
    let made = host.dispatch(
        &root,
        &pso,
        &Dispatch {
            roots,
            views: &views,
            samplers: &[],
            groups: plan.groups,
        },
    )?;
    // Every UAV in heap order, an alias with the words of the buffer it views.
    let mut by_slot = std::collections::HashMap::new();
    let mut made = made.into_iter();
    let mut out = Vec::new();
    for (i, v) in plan.views.iter().enumerate() {
        match v {
            PlanView::Typed { srv: false, .. } | PlanView::Raw(_, false) => {
                let words = made.next().ok_or("fewer UAVs than views")?;
                by_slot.insert(i, words.clone());
                out.push(words);
            }
            PlanView::Alias { of, .. } => out.push(by_slot.get(of).cloned().ok_or("alias before its view")?),
            _ => {}
        }
    }
    Ok(out)
}

#[test]
fn oracle_run() {
    let Some((out, _)) = dirs() else {
        eprintln!("VICTORY_ORACLE_DIR unset; skipped");
        return;
    };
    let warp = std::env::var("VICTORY_WARP").is_ok_and(|v| v == "1");
    let only = std::env::var("VICTORY_ORACLE_ONLY").ok();
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let host = Host::new(warp).expect("host device");
    eprintln!("adapter: {:?}", host.adapter_name());
    let mut paths: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "plan"))
        .filter(|p| only.as_ref().is_none_or(|o| stem(p).contains(o.as_str())))
        .collect();
    paths.sort();
    for path in paths {
        let plan = parse_plan(&std::fs::read_to_string(&path).unwrap());
        let s = stem(&path);
        let ext = if warp { "warp" } else { "gpu" };
        let text = match run_plan(&host, &plan) {
            Ok(uavs) => {
                let mut t = String::new();
                for (i, words) in uavs.iter().enumerate() {
                    write!(t, "uav {i}").unwrap();
                    for w in words {
                        write!(t, " {w:08x}").unwrap();
                    }
                    t.push('\n');
                }
                t
            }
            Err(e) => format!("error {e}\n"),
        };
        eprintln!(
            "{s}: {}",
            text.lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
        );
        std::fs::write(out.join(format!("{s}.{ext}")), text).unwrap();
    }
}
