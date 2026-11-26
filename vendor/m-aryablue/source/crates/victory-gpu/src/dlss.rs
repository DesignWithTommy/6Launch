//! dlss super resolution via nvidia's ngx (nvidia dlss sdk, github.com/NVIDIA/DLSS); feature `dlss`, switch VICTORY_DLSS.

use std::ffi::{CStr, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use windows::Win32::Graphics::Direct3D12::{ID3D12Device, ID3D12GraphicsCommandList, ID3D12Resource};
use windows::core::Interface;

mod ngx {
    use std::ffi::{c_char, c_void};

    pub type Result = u32;
    pub const SUCCESS: u32 = 1;
    pub const FEATURE_SUPER_SAMPLING: i32 = 1;
    pub const ENGINE_CUSTOM: i32 = 0;
    pub const VERSION_API: i32 = 0x15;

    pub const PERF_MAX_PERF: i32 = 0;
    pub const PERF_BALANCED: i32 = 1;
    pub const PERF_MAX_QUALITY: i32 = 2;
    pub const PERF_ULTRA_PERFORMANCE: i32 = 3;

    pub const FLAG_IS_HDR: i32 = 1 << 0;
    pub const FLAG_MV_LOW_RES: i32 = 1 << 1;
    pub const FLAG_MV_JITTERED: i32 = 1 << 2;
    pub const FLAG_DEPTH_INVERTED: i32 = 1 << 3;
    pub const FLAG_AUTO_EXPOSURE: i32 = 1 << 6;

    #[repr(C)]
    pub struct Parameter {
        _p: [u8; 0],
    }
    #[repr(C)]
    pub struct Handle {
        _p: [u8; 0],
    }

    #[repr(C)]
    pub struct PathListInfo {
        pub path: *const *const u16,
        pub length: u32,
    }
    pub type LogCallback = extern "C" fn(*const c_char, i32, i32);
    #[repr(C)]
    pub struct LoggingInfo {
        pub callback: Option<LogCallback>,
        pub minimum_level: i32,
        pub disable_other_sinks: bool,
    }
    #[repr(C)]
    pub struct FeatureCommonInfo {
        pub path_list: PathListInfo,
        pub internal: *mut c_void,
        pub logging: LoggingInfo,
    }

    pub type ParamCallback = unsafe extern "C" fn(*mut Parameter) -> Result;

    unsafe extern "C" {
        pub fn NVSDK_NGX_D3D12_Init_with_ProjectID(
            project_id: *const c_char,
            engine: i32,
            engine_version: *const c_char,
            app_data_path: *const u16,
            device: *mut c_void,
            info: *const FeatureCommonInfo,
            version: i32,
        ) -> Result;
        pub fn NVSDK_NGX_D3D12_Shutdown1(device: *mut c_void) -> Result;
        pub fn NVSDK_NGX_D3D12_GetCapabilityParameters(out: *mut *mut Parameter) -> Result;
        pub fn NVSDK_NGX_D3D12_DestroyParameters(params: *mut Parameter) -> Result;
        pub fn NVSDK_NGX_D3D12_CreateFeature(
            list: *mut c_void,
            feature: i32,
            params: *mut Parameter,
            out: *mut *mut Handle,
        ) -> Result;
        pub fn NVSDK_NGX_D3D12_EvaluateFeature_C(
            list: *mut c_void,
            handle: *const Handle,
            params: *const Parameter,
            progress: *const c_void,
        ) -> Result;
        pub fn NVSDK_NGX_D3D12_ReleaseFeature(handle: *mut Handle) -> Result;

        pub fn NVSDK_NGX_Parameter_SetUI(p: *mut Parameter, name: *const c_char, v: u32);
        pub fn NVSDK_NGX_Parameter_SetI(p: *mut Parameter, name: *const c_char, v: i32);
        pub fn NVSDK_NGX_Parameter_SetF(p: *mut Parameter, name: *const c_char, v: f32);
        pub fn NVSDK_NGX_Parameter_SetD3d12Resource(p: *mut Parameter, name: *const c_char, v: *mut c_void);
        pub fn NVSDK_NGX_Parameter_GetUI(p: *mut Parameter, name: *const c_char, v: *mut u32) -> Result;
        pub fn NVSDK_NGX_Parameter_GetI(p: *mut Parameter, name: *const c_char, v: *mut i32) -> Result;
        pub fn NVSDK_NGX_Parameter_GetVoidPointer(p: *mut Parameter, name: *const c_char, v: *mut *mut c_void) -> Result;
    }
}

const PROJECT_ID: &CStr = c"5c1f6a3e-0b7d-4e2a-9c41-7d2b8e6f1a90";

macro_rules! name {
    ($s:literal) => {
        concat!($s, "\0").as_ptr().cast::<c_char>()
    };
}

fn ok(what: &str, r: ngx::Result) -> Result<()> {
    if r == ngx::SUCCESS { Ok(()) } else { bail!("ngx {what} failed: {r:#010x}") }
}

/// The DLSS quality mode, from VICTORY_DLSS=quality|balanced|performance|ultra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DlssQuality {
    Quality,
    Balanced,
    Performance,
    UltraPerformance,
}

impl DlssQuality {
    /// none when VICTORY_DLSS is unset, empty, 0 or off.
    pub fn from_env() -> Option<Self> {
        Self::parse(&std::env::var("VICTORY_DLSS").ok()?)
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "quality" | "1" => Some(Self::Quality),
            "balanced" => Some(Self::Balanced),
            "performance" | "perf" => Some(Self::Performance),
            "ultra" | "ultra_performance" | "ultraperformance" => Some(Self::UltraPerformance),
            _ => None,
        }
    }

    fn ngx(self) -> i32 {
        match self {
            Self::Quality => ngx::PERF_MAX_QUALITY,
            Self::Balanced => ngx::PERF_BALANCED,
            Self::Performance => ngx::PERF_MAX_PERF,
            Self::UltraPerformance => ngx::PERF_ULTRA_PERFORMANCE,
        }
    }
}

/// Fixed at feature creation; a change recreates the feature on the next upscale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DlssSettings {
    pub quality: DlssQuality,
    pub hdr: bool,
    pub depth_inverted: bool,
    pub mv_jittered: bool,
    pub auto_exposure: bool,
    pub mv_low_res: bool,
}

impl DlssSettings {
    pub fn new(quality: DlssQuality) -> Self {
        Self { quality, hdr: false, depth_inverted: false, mv_jittered: false, auto_exposure: false, mv_low_res: true }
    }

    /// from ffx context flags: 0x1 hdr, 0x2 display-res mvs, 0x4 jittered mvs, 0x8 depth inverted, 0x20 auto exposure.
    pub fn from_ffx_flags(quality: DlssQuality, flags: u32) -> Self {
        Self {
            quality,
            hdr: flags & 0x1 != 0,
            mv_low_res: flags & 0x2 == 0,
            mv_jittered: flags & 0x4 != 0,
            depth_inverted: flags & 0x8 != 0,
            auto_exposure: flags & 0x20 != 0,
        }
    }

    fn flags(&self) -> i32 {
        let mut f = 0;
        if self.mv_low_res {
            f |= ngx::FLAG_MV_LOW_RES;
        }
        if self.hdr {
            f |= ngx::FLAG_IS_HDR;
        }
        if self.mv_jittered {
            f |= ngx::FLAG_MV_JITTERED;
        }
        if self.depth_inverted {
            f |= ngx::FLAG_DEPTH_INVERTED;
        }
        if self.auto_exposure {
            f |= ngx::FLAG_AUTO_EXPOSURE;
        }
        f
    }
}

/// Render sizes DLSS wants for one output size and quality mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSize {
    pub optimal: [u32; 2],
    pub min: [u32; 2],
    pub max: [u32; 2],
}

/// inputs in pixel|non-pixel shader-resource state, output in unordered access; ngx changes the list's heaps/root state.
pub struct UpscaleInputs<'a> {
    pub color: &'a ID3D12Resource,
    pub depth: &'a ID3D12Resource,
    pub motion_vectors: &'a ID3D12Resource,
    pub exposure: Option<&'a ID3D12Resource>,
    pub output: &'a ID3D12Resource,
    /// jitter in render pixels, as the title applied it
    pub jitter: [f32; 2],
    /// scale taking the title's vectors to render pixels (fsr's motion vector scale)
    pub mv_scale: [f32; 2],
    /// the rendered part of `color` (top-left), for dynamic resolution
    pub render_size: [u32; 2],
    pub reset: bool,
    pub pre_exposure: f32,
    pub frame_time_ms: f32,
}

struct Feature {
    handle: *mut ngx::Handle,
    input: [u32; 2],
    output: [u32; 2],
    settings: DlssSettings,
}

static LIVE: AtomicBool = AtomicBool::new(false);

/// DLSS Super Resolution on one host device.
pub struct DlssUpscaler {
    device: ID3D12Device,
    params: *mut ngx::Parameter,
    feature: Option<Feature>,
    settings: DlssSettings,
    _paths: Box<(Vec<u16>, [*const u16; 1], ngx::FeatureCommonInfo)>,
}

// SAFETY: ngx's d3d12 calls are made from one thread at a time through &mut self.
unsafe impl Send for DlssUpscaler {}

extern "C" fn log_line(msg: *const c_char, _level: i32, _feature: i32) {
    if !msg.is_null() {
        // SAFETY: ngx passes a nul-terminated line that lives for the call.
        let line = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
        tracing::debug!(target: "victory::dlss", "{}", line.trim_end());
    }
}

fn dll_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("VICTORY_DLSS_DLL_DIR") {
        return Some(d.into());
    }
    let sdk = std::env::var_os("VICTORY_DLSS_SDK")
        .map(PathBuf::from)
        .or_else(|| option_env!("VICTORY_DLSS_SDK_BUILT").map(PathBuf::from))?;
    Some(sdk.join("lib").join("Windows_x86_64").join("rel"))
}

fn wide(p: &std::path::Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

impl DlssUpscaler {
    /// none (and nothing loaded) unless VICTORY_DLSS names a mode.
    pub fn from_env(device: &ID3D12Device) -> Option<Result<Self>> {
        DlssQuality::from_env().map(|q| Self::new(device, DlssSettings::new(q)))
    }

    /// inits ngx on the device and checks dlss is available there.
    pub fn new(device: &ID3D12Device, settings: DlssSettings) -> Result<Self> {
        if LIVE.swap(true, Ordering::AcqRel) {
            bail!("one dlss upscaler at a time");
        }
        match Self::init(device, settings) {
            Ok(u) => Ok(u),
            Err(e) => {
                LIVE.store(false, Ordering::Release);
                Err(e)
            }
        }
    }

    fn init(device: &ID3D12Device, settings: DlssSettings) -> Result<Self> {
        let dir = dll_dir().context("no VICTORY_DLSS_SDK or VICTORY_DLSS_DLL_DIR")?;
        if !dir.join("nvngx_dlss.dll").is_file() {
            bail!("no nvngx_dlss.dll in {}", dir.display());
        }
        let data = std::env::var_os("VICTORY_DLSS_LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("victory-ngx"));
        std::fs::create_dir_all(&data).with_context(|| format!("ngx data dir {}", data.display()))?;
        let verbose = std::env::var("VICTORY_DLSS_LOG").is_ok_and(|v| v == "1");

        let mut paths = Box::new((
            wide(&dir),
            [std::ptr::null(); 1],
            ngx::FeatureCommonInfo {
                path_list: ngx::PathListInfo { path: std::ptr::null(), length: 1 },
                internal: std::ptr::null_mut(),
                logging: ngx::LoggingInfo {
                    callback: Some(log_line),
                    minimum_level: if verbose { 1 } else { 0 },
                    disable_other_sinks: false,
                },
            },
        ));
        paths.1[0] = paths.0.as_ptr();
        paths.2.path_list.path = paths.1.as_ptr();
        let data_w = wide(&data);

        // SAFETY: the device outlives ngx (we hold a reference until shutdown); paths live in the box.
        unsafe {
            ok(
                "init",
                ngx::NVSDK_NGX_D3D12_Init_with_ProjectID(
                    PROJECT_ID.as_ptr(),
                    ngx::ENGINE_CUSTOM,
                    c"1.0".as_ptr(),
                    data_w.as_ptr(),
                    device.as_raw(),
                    &paths.2,
                    ngx::VERSION_API,
                ),
            )?;
        }
        let mut params = std::ptr::null_mut();
        // SAFETY: ngx is initialised on this device.
        let caps = unsafe { ngx::NVSDK_NGX_D3D12_GetCapabilityParameters(&mut params) };
        if caps != ngx::SUCCESS || params.is_null() {
            // SAFETY: undo the init above.
            unsafe { ngx::NVSDK_NGX_D3D12_Shutdown1(device.as_raw()) };
            bail!("ngx capability parameters failed: {caps:#010x}");
        }
        let up = Self { device: device.clone(), params, feature: None, settings, _paths: paths };
        let (available, why) = up.availability();
        if !available {
            bail!("dlss not available: {why}");
        }
        tracing::info!(target: "victory::dlss", quality = ?settings.quality, "dlss ready (nvidia dlss sdk, ngx)");
        Ok(up)
    }

    /// whether dlss sr runs here, and why not.
    pub fn availability(&self) -> (bool, String) {
        let (mut avail, mut needs_driver, mut major, mut minor, mut init) = (0i32, 0i32, 0u32, 0u32, 0i32);
        // SAFETY: params is ngx's capability block; missing names leave the zeros.
        unsafe {
            ngx::NVSDK_NGX_Parameter_GetI(self.params, name!("SuperSampling.Available"), &mut avail);
            ngx::NVSDK_NGX_Parameter_GetI(self.params, name!("SuperSampling.NeedsUpdatedDriver"), &mut needs_driver);
            ngx::NVSDK_NGX_Parameter_GetUI(self.params, name!("SuperSampling.MinDriverVersionMajor"), &mut major);
            ngx::NVSDK_NGX_Parameter_GetUI(self.params, name!("SuperSampling.MinDriverVersionMinor"), &mut minor);
            ngx::NVSDK_NGX_Parameter_GetI(self.params, name!("SuperSampling.FeatureInitResult"), &mut init);
        }
        if avail != 0 {
            return (true, String::new());
        }
        let why = if needs_driver != 0 {
            format!("driver too old, needs {major}.{minor}")
        } else {
            format!("feature init result {:#010x}", init as u32)
        };
        (false, why)
    }

    pub fn settings(&self) -> DlssSettings {
        self.settings
    }

    /// takes effect on the next upscale (the feature is recreated).
    pub fn set_settings(&mut self, settings: DlssSettings) {
        self.settings = settings;
    }

    /// render sizes for an output size at the current quality mode.
    pub fn optimal_render_size(&self, output: [u32; 2]) -> Result<RenderSize> {
        let p = self.params;
        let mut cb: *mut c_void = std::ptr::null_mut();
        // SAFETY: ngx's capability block holds the callback; we call it the way the sdk helper does.
        unsafe {
            ngx::NVSDK_NGX_Parameter_GetVoidPointer(p, name!("DLSSOptimalSettingsCallback"), &mut cb);
            if cb.is_null() {
                bail!("dlss optimal-settings callback missing (old dlss dll)");
            }
            let cb: ngx::ParamCallback = std::mem::transmute(cb);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("Width"), output[0]);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("Height"), output[1]);
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("PerfQualityValue"), self.settings.quality.ngx());
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("RTXValue"), 0);
            ok("optimal settings", cb(p))?;
            let mut optimal = [0u32; 2];
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("OutWidth"), &mut optimal[0]);
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("OutHeight"), &mut optimal[1]);
            let (mut min, mut max) = (optimal, optimal);
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("DLSS.Get.Dynamic.Max.Render.Width"), &mut max[0]);
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("DLSS.Get.Dynamic.Max.Render.Height"), &mut max[1]);
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("DLSS.Get.Dynamic.Min.Render.Width"), &mut min[0]);
            ngx::NVSDK_NGX_Parameter_GetUI(p, name!("DLSS.Get.Dynamic.Min.Render.Height"), &mut min[1]);
            if optimal[0] == 0 || optimal[1] == 0 {
                bail!("dlss gave no render size for {}x{}", output[0], output[1]);
            }
            Ok(RenderSize { optimal, min, max })
        }
    }

    fn ensure_feature(&mut self, list: &ID3D12GraphicsCommandList, input: [u32; 2], output: [u32; 2]) -> Result<()> {
        let want = (input, output, self.settings);
        if let Some(f) = &self.feature
            && (f.input, f.output, f.settings) == want
        {
            return Ok(());
        }
        self.release_feature();
        let p = self.params;
        let mut handle = std::ptr::null_mut();
        // SAFETY: plain parameter writes and a create recorded on the caller's open list.
        unsafe {
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("CreationNodeMask"), 1);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("VisibilityNodeMask"), 1);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("Width"), input[0]);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("Height"), input[1]);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("OutWidth"), output[0]);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("OutHeight"), output[1]);
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("PerfQualityValue"), self.settings.quality.ngx());
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("DLSS.Feature.Create.Flags"), self.settings.flags());
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("DLSS.Enable.Output.Subrects"), 0);
            ok(
                "create dlss",
                ngx::NVSDK_NGX_D3D12_CreateFeature(list.as_raw(), ngx::FEATURE_SUPER_SAMPLING, p, &mut handle),
            )?;
        }
        tracing::info!(target: "victory::dlss", "dlss feature {}x{} -> {}x{}", input[0], input[1], output[0], output[1]);
        self.feature = Some(Feature { handle, input, output, settings: self.settings });
        Ok(())
    }

    /// records dlss into an open direct list; creates or recreates the feature as sizes change.
    pub fn upscale(&mut self, list: &ID3D12GraphicsCommandList, inputs: &UpscaleInputs) -> Result<()> {
        // SAFETY: reading resource descriptions.
        let (cd, od) = unsafe { (inputs.color.GetDesc(), inputs.output.GetDesc()) };
        let input = [cd.Width as u32, cd.Height];
        let output = [od.Width as u32, od.Height];
        let render = [inputs.render_size[0].min(input[0]), inputs.render_size[1].min(input[1])];
        if render[0] == 0 || render[1] == 0 {
            bail!("dlss render size is zero");
        }
        self.ensure_feature(list, input, output)?;
        let handle = self.feature.as_ref().map(|f| f.handle).unwrap_or(std::ptr::null_mut());
        let p = self.params;
        let exposure = inputs.exposure.map_or(std::ptr::null_mut(), |r| r.as_raw());
        let mv = |v: f32| if v == 0.0 { 1.0 } else { v };
        // SAFETY: the resources are alive for the call; ngx records into the caller's list.
        unsafe {
            ngx::NVSDK_NGX_Parameter_SetD3d12Resource(p, name!("Color"), inputs.color.as_raw());
            ngx::NVSDK_NGX_Parameter_SetD3d12Resource(p, name!("Output"), inputs.output.as_raw());
            ngx::NVSDK_NGX_Parameter_SetD3d12Resource(p, name!("Depth"), inputs.depth.as_raw());
            ngx::NVSDK_NGX_Parameter_SetD3d12Resource(p, name!("MotionVectors"), inputs.motion_vectors.as_raw());
            ngx::NVSDK_NGX_Parameter_SetD3d12Resource(p, name!("ExposureTexture"), exposure);
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("Jitter.Offset.X"), inputs.jitter[0]);
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("Jitter.Offset.Y"), inputs.jitter[1]);
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("MV.Scale.X"), mv(inputs.mv_scale[0]));
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("MV.Scale.Y"), mv(inputs.mv_scale[1]));
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("Sharpness"), 0.0);
            ngx::NVSDK_NGX_Parameter_SetI(p, name!("Reset"), i32::from(inputs.reset));
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.Color.Subrect.Base.X"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.Color.Subrect.Base.Y"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.Depth.Subrect.Base.X"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.Depth.Subrect.Base.Y"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.MV.Subrect.Base.X"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Input.MV.Subrect.Base.Y"), 0);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Render.Subrect.Dimensions.Width"), render[0]);
            ngx::NVSDK_NGX_Parameter_SetUI(p, name!("DLSS.Render.Subrect.Dimensions.Height"), render[1]);
            let pre = if inputs.pre_exposure == 0.0 { 1.0 } else { inputs.pre_exposure };
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("DLSS.Pre.Exposure"), pre);
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("DLSS.Exposure.Scale"), 1.0);
            ngx::NVSDK_NGX_Parameter_SetF(p, name!("FrameTimeDeltaInMsec"), inputs.frame_time_ms);
            ok(
                "evaluate dlss",
                ngx::NVSDK_NGX_D3D12_EvaluateFeature_C(list.as_raw(), handle, p, std::ptr::null()),
            )
        }
    }

    /// drops the feature; only once the gpu is done with lists that used it.
    pub fn release_feature(&mut self) {
        if let Some(f) = self.feature.take() {
            // SAFETY: the handle came from create and is released once.
            let r = unsafe { ngx::NVSDK_NGX_D3D12_ReleaseFeature(f.handle) };
            if r != ngx::SUCCESS {
                tracing::warn!(target: "victory::dlss", "ngx release feature: {r:#010x}");
            }
        }
    }
}

impl Drop for DlssUpscaler {
    fn drop(&mut self) {
        self.release_feature();
        // SAFETY: params came from ngx; shutdown matches the init on this device.
        unsafe {
            ngx::NVSDK_NGX_D3D12_DestroyParameters(self.params);
            ngx::NVSDK_NGX_D3D12_Shutdown1(self.device.as_raw());
        }
        LIVE.store(false, Ordering::Release);
    }
}

/// s9's upscaler shape: setup/dispatch/release return bools, ngx is brought up on first setup.
pub struct DlssBackend {
    quality: DlssQuality,
    up: Option<DlssUpscaler>,
    failed: bool,
}

impl DlssBackend {
    /// none unless VICTORY_DLSS names a mode.
    pub fn from_env() -> Option<Self> {
        DlssQuality::from_env().map(|quality| Self { quality, up: None, failed: false })
    }

    pub fn name(&self) -> &'static str {
        "dlss"
    }

    pub fn setup(&mut self, device: &ID3D12Device, ffx_flags: u32) -> bool {
        if self.failed {
            return false;
        }
        let settings = DlssSettings::from_ffx_flags(self.quality, ffx_flags);
        if let Some(up) = &mut self.up {
            up.set_settings(settings);
            return true;
        }
        match DlssUpscaler::new(device, settings) {
            Ok(up) => {
                self.up = Some(up);
                true
            }
            Err(e) => {
                tracing::warn!(target: "victory::dlss", "dlss off: {e:#}");
                self.failed = true;
                false
            }
        }
    }

    pub fn dispatch(&mut self, list: &ID3D12GraphicsCommandList, inputs: &UpscaleInputs) -> bool {
        let Some(up) = &mut self.up else { return false };
        match up.upscale(list, inputs) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(target: "victory::dlss", "dlss skipped: {e:#}");
                false
            }
        }
    }

    pub fn optimal_render_size(&self, output: [u32; 2]) -> Option<RenderSize> {
        self.up.as_ref()?.optimal_render_size(output).ok()
    }

    /// only once the gpu is done with lists that used the feature.
    pub fn release(&mut self) {
        if let Some(up) = &mut self.up {
            up.release_feature();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;

    #[test]
    fn quality_names() {
        assert_eq!(DlssQuality::parse("quality"), Some(DlssQuality::Quality));
        assert_eq!(DlssQuality::parse("Balanced"), Some(DlssQuality::Balanced));
        assert_eq!(DlssQuality::parse("performance"), Some(DlssQuality::Performance));
        assert_eq!(DlssQuality::parse("ultra"), Some(DlssQuality::UltraPerformance));
        assert_eq!(DlssQuality::parse("off"), None);
        assert_eq!(DlssQuality::parse(""), None);
        let s = DlssSettings::from_ffx_flags(DlssQuality::Quality, 0x1 | 0x8);
        assert!(s.hdr && s.depth_inverted && s.mv_low_res && !s.mv_jittered);
        assert!(!DlssSettings::from_ffx_flags(DlssQuality::Quality, 0x2).mv_low_res);
    }

    fn nvidia_device() -> Option<(ID3D12Device, String)> {
        // SAFETY: plain dxgi/d3d12 queries.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            for i in 0.. {
                let adapter = factory.EnumAdapters1(i).ok()?;
                let desc = adapter.GetDesc1().ok()?;
                if desc.VendorId != 0x10de || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let mut device: Option<ID3D12Device> = None;
                if D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).is_ok() {
                    let name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_owned();
                    return device.map(|d| (d, name));
                }
            }
            None
        }
    }

    fn texture(
        device: &ID3D12Device,
        size: [u32; 2],
        format: DXGI_FORMAT,
        flags: D3D12_RESOURCE_FLAGS,
        state: D3D12_RESOURCE_STATES,
    ) -> ID3D12Resource {
        let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: u64::from(size[0]),
            Height: size[1],
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Flags: flags,
            ..Default::default()
        };
        let mut out: Option<ID3D12Resource> = None;
        // SAFETY: plain texture creation (zero-filled: the synthetic inputs).
        unsafe {
            device.CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &desc, state, None, &mut out).unwrap();
        }
        out.unwrap()
    }

    // smoke test: init, availability, create 1280x720 -> 2560x1440, one evaluate, release
    #[test]
    fn dlss_smoke_on_nvidia() {
        let Some((device, card)) = nvidia_device() else {
            eprintln!("dlss smoke: skipped, no nvidia gpu");
            return;
        };
        if dll_dir().is_none_or(|d| !d.join("nvngx_dlss.dll").is_file()) {
            eprintln!("dlss smoke: skipped, no nvngx_dlss.dll (set VICTORY_DLSS_SDK)");
            return;
        }
        eprintln!("dlss smoke: {card}");
        let mut up = DlssUpscaler::new(&device, DlssSettings::new(DlssQuality::Quality)).expect("ngx init");
        assert!(up.availability().0);
        for q in [DlssQuality::Quality, DlssQuality::Balanced, DlssQuality::Performance, DlssQuality::UltraPerformance]
        {
            up.set_settings(DlssSettings::new(q));
            let r = up.optimal_render_size([2560, 1440]).expect("optimal settings");
            eprintln!("dlss smoke: {q:?} 2560x1440 renders {:?} (min {:?} max {:?})", r.optimal, r.min, r.max);
        }
        up.set_settings(DlssSettings::new(DlssQuality::Performance));

        let read = D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE | D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE;
        let low = [1280, 720];
        let color = texture(&device, low, DXGI_FORMAT_R16G16B16A16_FLOAT, D3D12_RESOURCE_FLAG_NONE, read);
        let depth = texture(&device, low, DXGI_FORMAT_R32_FLOAT, D3D12_RESOURCE_FLAG_NONE, read);
        let mv = texture(&device, low, DXGI_FORMAT_R16G16_FLOAT, D3D12_RESOURCE_FLAG_NONE, read);
        let output = texture(
            &device,
            [2560, 1440],
            DXGI_FORMAT_R16G16B16A16_FLOAT,
            D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
        );

        // SAFETY: plain object creation and one submit we wait for.
        unsafe {
            let queue: ID3D12CommandQueue = device
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC { Type: D3D12_COMMAND_LIST_TYPE_DIRECT, ..Default::default() })
                .unwrap();
            let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();
            let list: ID3D12GraphicsCommandList =
                device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None).unwrap();
            let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap();

            let inputs = UpscaleInputs {
                color: &color,
                depth: &depth,
                motion_vectors: &mv,
                exposure: None,
                output: &output,
                jitter: [0.25, -0.25],
                mv_scale: [1.0, 1.0],
                render_size: low,
                reset: true,
                pre_exposure: 1.0,
                frame_time_ms: 16.6,
            };
            up.upscale(&list, &inputs).expect("dlss evaluate");
            list.Close().unwrap();
            queue.ExecuteCommandLists(&[Some(list.cast::<ID3D12CommandList>().unwrap())]);
            queue.Signal(&fence, 1).unwrap();
            fence.SetEventOnCompletion(1, HANDLE::default()).unwrap();
            device.GetDeviceRemovedReason().expect("device lost after dlss");
        }
        up.release_feature();
        drop(up);
        eprintln!("dlss smoke: ok");
    }
}
