//! The launch options of the Play page and the environment each one sets. Every option that
//! reaches VIctory goes through the one table in `env_pairs`, and `ENV_NAMES` lists the variable
//! names it may use: a test checks each of them against VIctory's own sources in this tree, so an
//! option never names a variable nothing reads.
//!
//! Not offered: the Series S console profile (`victory run --console series-s`, an argument, not a
//! variable). Gone with the Neptune base, which had them: the F10 stats overlay (VIctory's panel,
//! `VICTORY_PANEL=1`, is in diagnostics builds only), the frame-time CSV and the texture size cap.

use serde::{Deserialize, Serialize};

use crate::env::Pair;

pub const DISPLAY_MODE: &str = "VICTORY_DISPLAY_MODE";
pub const WINDOW_MODE: &str = "VICTORY_WINDOW_MODE";
pub const KEYBOARD: &str = "VICTORY_KEYBOARD";
pub const LOG: &str = "VICTORY_LOG";
/// Taps Enter (also the first pad's A) on the start screens for 150 seconds: the same script as
/// `--auto-a` (`anyxboxx::run::auto_a_keys`).
pub const KEYS: &str = "VICTORY_KEYS";
/// Every variable an option may set (the preset's own are in `Preset::overrides`); the tests
/// check them against VIctory's sources.
#[cfg(test)]
pub const ENV_NAMES: [&str; 5] = [DISPLAY_MODE, WINDOW_MODE, KEYBOARD, LOG, KEYS];

/// VIctory accepts a display size inside these bounds and falls back to 4K outside them.
const WIDTH_RANGE: (u32, u32) = (640, 7680);
const HEIGHT_RANGE: (u32, u32) = (360, 4320);

/// VIctory's own room left for textures when nothing says otherwise (`VICTORY_TEXTURE_ROOM_PERCENT`,
/// `victory-gpu` `resource.rs`).
#[cfg(test)]
pub const DEFAULT_TEXTURE_ROOM_PERCENT: u32 = 90;

/// The "8 GB card" preset: VIctory's buffer copies on the card capped at 512 MB (by default they
/// scale with the card: 512 MB up to a 12 GB budget, 2048 MB on 16 GB cards) and textures given 85 %
/// of the card's budget instead of 90 %, so that the card stays under the budget Windows gives the
/// game. Draw-data windows stay in the PC's memory on this base by default
/// (`VICTORY_CARD_WINDOWS_MB` unset). The Neptune base's texture size cap
/// (`VICTORY_TEXTURE_MAX_SIZE`) is not in this base. Not measured on this base.
pub const LOW_VRAM: &[(&str, &str)] = &[
    ("VICTORY_CARD_BUFFERS_MB", "512"),
    ("VICTORY_TEXTURE_ROOM_PERCENT", "85"),
];

/// The "12 GB card" preset: buffer copies on the card capped at 1024 MB, textures at VIctory's
/// default room. Not measured on a 12 GB card yet.
pub const MID_VRAM: &[(&str, &str)] = &[
    ("VICTORY_CARD_BUFFERS_MB", "1024"),
    ("VICTORY_TEXTURE_ROOM_PERCENT", "90"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    Fast,
    Quality,
    /// For cards with 8 GB of video memory or less.
    LowVram,
    /// For cards with 12 GB of video memory.
    MidVram,
}

impl Preset {
    pub const ALL: [Preset; 4] = [
        Preset::Fast,
        Preset::Quality,
        Preset::MidVram,
        Preset::LowVram,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Fast => "Fast",
            Preset::Quality => "Quality",
            Preset::LowVram => "8 GB card",
            Preset::MidVram => "12 GB card",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Preset::Fast => "The profile as measured: the best speed known so far.",
            Preset::Quality => {
                "Picture first: windows and buffers read fresh each frame, fewer copies on the card. Slower; not measured on this VIctory base yet."
            }
            Preset::LowVram => {
                "For 8 GB cards: smaller buffer reserves on the card and less room for textures, so the card stays within its budget instead of spilling into the PC's memory. Pair it with the 1920x1080 display size. Not measured on this VIctory base yet."
            }
            Preset::MidVram => {
                "For 12 GB cards: buffer reserves on the card capped at 1024 MB. Not measured on a 12 GB card yet."
            }
        }
    }

    /// The variables this preset adds on top of the profile. Fast is the profile itself; Quality
    /// is main's `-Quality` set (`anyxboxx::run::QUALITY`).
    pub fn overrides(self) -> Vec<Pair> {
        match self {
            Preset::Fast => Vec::new(),
            Preset::Quality => anyxboxx::run::QUALITY
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            Preset::LowVram => LOW_VRAM
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            Preset::MidVram => MID_VRAM
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// VIctory's own default (the 4K display the console reports).
    #[default]
    Default,
    P720,
    P900,
    P1080,
    P1440,
    P2160,
    Custom,
}

impl Resolution {
    pub const ALL: [Resolution; 7] = [
        Resolution::Default,
        Resolution::P720,
        Resolution::P900,
        Resolution::P1080,
        Resolution::P1440,
        Resolution::P2160,
        Resolution::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Resolution::Default => "Default",
            Resolution::P720 => "1280x720",
            Resolution::P900 => "1600x900",
            Resolution::P1080 => "1920x1080",
            Resolution::P1440 => "2560x1440",
            Resolution::P2160 => "3840x2160",
            Resolution::Custom => "Custom",
        }
    }

    fn fixed_size(self) -> Option<(u32, u32)> {
        match self {
            Resolution::P720 => Some((1280, 720)),
            Resolution::P900 => Some((1600, 900)),
            Resolution::P1080 => Some((1920, 1080)),
            Resolution::P1440 => Some((2560, 1440)),
            Resolution::P2160 => Some((3840, 2160)),
            Resolution::Default | Resolution::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Windowed,
    Borderless,
}

impl WindowMode {
    pub const ALL: [WindowMode; 2] = [WindowMode::Windowed, WindowMode::Borderless];

    pub fn label(self) -> &'static str {
        match self {
            WindowMode::Windowed => "Windowed",
            WindowMode::Borderless => "Borderless full screen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keyboard {
    /// By physical key position: ZQSD works on AZERTY with nothing to set.
    #[default]
    Auto,
    Qwerty,
    Azerty,
    Qwertz,
}

impl Keyboard {
    pub const ALL: [Keyboard; 4] = [
        Keyboard::Auto,
        Keyboard::Qwerty,
        Keyboard::Azerty,
        Keyboard::Qwertz,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Keyboard::Auto => "Auto (physical keys)",
            Keyboard::Qwerty => "QWERTY",
            Keyboard::Azerty => "AZERTY",
            Keyboard::Qwertz => "QWERTZ",
        }
    }

    /// The letters of the left stick, up/left/down/right, as printed on that keyboard.
    pub fn move_keys(self) -> &'static str {
        match self {
            Keyboard::Azerty => "Z Q S D",
            Keyboard::Auto => "W A S D positions (Z Q S D on AZERTY)",
            Keyboard::Qwerty | Keyboard::Qwertz => "W A S D",
        }
    }

    /// The letters of the bumper and trigger keys (left/right) as printed on that keyboard.
    pub fn shoulder_keys(self) -> &'static str {
        match self {
            Keyboard::Azerty => "Bumpers A E, triggers W C",
            Keyboard::Qwertz => "Bumpers Q E, triggers Y C",
            Keyboard::Auto | Keyboard::Qwerty => "Bumpers Q E, triggers Z C",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    #[default]
    Default,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub const ALL: [LogLevel; 4] = [
        LogLevel::Default,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Default => "Default",
            LogLevel::Warn => "Warnings only",
            LogLevel::Info => "Info",
            LogLevel::Debug => "Debug (slow)",
        }
    }
}

/// What the Play page's cards choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchOptions {
    pub preset: Preset,
    pub resolution: Resolution,
    pub custom_width: u32,
    pub custom_height: u32,
    pub window: WindowMode,
    pub keyboard: Keyboard,
    pub log: LogLevel,
    /// Translate the shaders into the cache before starting the game.
    pub warm_first: bool,
    /// Taps Enter (and A) through the start screens for 150 seconds.
    pub auto_a: bool,
    /// NVIDIA Smooth Motion (driver frame generation) for victory.exe: written into the driver's
    /// profile before each launch (`smooth_motion`), not into VIctory's environment.
    pub smooth_motion: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        LaunchOptions {
            preset: Preset::Fast,
            resolution: Resolution::Default,
            custom_width: 1920,
            custom_height: 1080,
            window: WindowMode::Windowed,
            keyboard: Keyboard::Auto,
            log: LogLevel::Default,
            warm_first: false,
            auto_a: false,
            smooth_motion: false,
        }
    }
}

impl LaunchOptions {
    /// The display size to report, `None` for VIctory's default (or an out-of-range custom size).
    pub fn size(&self) -> Option<(u32, u32)> {
        match self.resolution {
            Resolution::Custom => self
                .custom_valid()
                .then_some((self.custom_width, self.custom_height)),
            other => other.fixed_size(),
        }
    }

    /// Whether the custom size is one VIctory accepts.
    pub fn custom_valid(&self) -> bool {
        (WIDTH_RANGE.0..=WIDTH_RANGE.1).contains(&self.custom_width)
            && (HEIGHT_RANGE.0..=HEIGHT_RANGE.1).contains(&self.custom_height)
    }

    /// The variables the options set: the preset's, then one table row per option that left its
    /// default. The user's own overrides go after these and win.
    pub fn env_pairs(&self) -> Vec<Pair> {
        let mut pairs = self.preset.overrides();
        if let Some((w, h)) = self.size() {
            pairs.push((DISPLAY_MODE.to_string(), format!("{w}x{h}")));
        }
        if self.window == WindowMode::Borderless {
            pairs.push((WINDOW_MODE.to_string(), "borderless".to_string()));
        }
        let keyboard = match self.keyboard {
            Keyboard::Auto => None,
            Keyboard::Qwerty => Some("qwerty"),
            Keyboard::Azerty => Some("azerty"),
            Keyboard::Qwertz => Some("qwertz"),
        };
        if let Some(value) = keyboard {
            pairs.push((KEYBOARD.to_string(), value.to_string()));
        }
        let log = match self.log {
            LogLevel::Default => None,
            LogLevel::Warn => Some("warn"),
            LogLevel::Info => Some("info"),
            LogLevel::Debug => Some("debug"),
        };
        if let Some(value) = log {
            pairs.push((LOG.to_string(), value.to_string()));
        }
        if self.auto_a {
            pairs.push((KEYS.to_string(), anyxboxx::run::auto_a_keys()));
        }
        pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_defaults_set_nothing() {
        assert!(LaunchOptions::default().env_pairs().is_empty());
    }

    #[test]
    fn every_choice_maps_to_its_variable() {
        let options = LaunchOptions {
            preset: Preset::Quality,
            resolution: Resolution::P1440,
            window: WindowMode::Borderless,
            keyboard: Keyboard::Azerty,
            log: LogLevel::Debug,
            ..LaunchOptions::default()
        };
        let pairs = options.env_pairs();
        let get = |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get(DISPLAY_MODE), Some("2560x1440"));
        assert_eq!(get(WINDOW_MODE), Some("borderless"));
        assert_eq!(get(KEYBOARD), Some("azerty"));
        assert_eq!(get(LOG), Some("debug"));
        assert_eq!(get("VICTORY_CARD_WINDOWS_MB"), Some("0"));
        assert_eq!(get("VICTORY_TEXTURE_STAGE_MB"), Some("192"));
    }

    #[test]
    fn auto_a_is_the_cli_s_key_script() {
        let options = LaunchOptions {
            auto_a: true,
            ..LaunchOptions::default()
        };
        let pairs = options.env_pairs();
        assert_eq!(
            pairs,
            vec![(KEYS.to_string(), anyxboxx::run::auto_a_keys())]
        );
        assert!(!LaunchOptions::default().auto_a);
    }

    #[test]
    fn quality_is_exactly_mains_quality_set() {
        let quality = Preset::Quality.overrides();
        let expected: Vec<Pair> = anyxboxx::run::QUALITY
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        assert_eq!(quality, expected);
        assert_eq!(quality.len(), 3);
    }

    #[test]
    fn the_card_presets_shrink_what_the_card_holds_8_gb_below_12_gb() {
        let value = |preset: Preset, name: &str| {
            preset
                .overrides()
                .into_iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.parse::<u32>().expect("a number"))
                .expect("set by the preset")
        };
        assert!(
            value(Preset::LowVram, "VICTORY_CARD_BUFFERS_MB")
                < value(Preset::MidVram, "VICTORY_CARD_BUFFERS_MB")
        );
        assert!(
            value(Preset::LowVram, "VICTORY_TEXTURE_ROOM_PERCENT") < DEFAULT_TEXTURE_ROOM_PERCENT
        );
        assert!(
            value(Preset::MidVram, "VICTORY_TEXTURE_ROOM_PERCENT") <= DEFAULT_TEXTURE_ROOM_PERCENT
        );
        // Neither preset reserves draw-data windows on the card: this base keeps none by default.
        for preset in [Preset::LowVram, Preset::MidVram] {
            assert!(
                preset
                    .overrides()
                    .iter()
                    .all(|(k, _)| k != "VICTORY_CARD_WINDOWS_MB")
            );
        }
        // The profiles leave both reserves to VIctory.
        assert!(
            anyxboxx::run::TUNED
                .iter()
                .all(|(k, _)| !k.starts_with("VICTORY_CARD_BUFFERS_MB")
                    && *k != "VICTORY_TEXTURE_ROOM_PERCENT")
        );
        assert!(Preset::ALL.contains(&Preset::LowVram) && Preset::ALL.contains(&Preset::MidVram));
    }

    #[test]
    fn a_custom_size_is_checked_against_victorys_bounds() {
        let mut options = LaunchOptions {
            resolution: Resolution::Custom,
            custom_width: 1366,
            custom_height: 768,
            ..LaunchOptions::default()
        };
        assert_eq!(options.size(), Some((1366, 768)));
        options.custom_width = 100;
        assert_eq!(options.size(), None);
        assert!(options.env_pairs().is_empty());
        options.custom_width = 1366;
        options.custom_height = 5000;
        assert!(!options.custom_valid());
    }

    #[test]
    fn the_legend_shows_the_keys_of_the_layout() {
        assert_eq!(Keyboard::Azerty.move_keys(), "Z Q S D");
        assert_eq!(Keyboard::Qwerty.move_keys(), "W A S D");
        assert!(Keyboard::Auto.move_keys().contains("AZERTY"));
        assert!(Keyboard::Qwertz.shoulder_keys().contains("Y C"));
    }

    #[test]
    fn every_variable_an_option_names_exists_in_victorys_sources() {
        let victory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("third_party")
            .join("victory")
            .join("crates");
        if !victory.is_dir() {
            return;
        }
        let mut sources = String::new();
        collect_rs(&victory, &mut sources);
        let preset_names: Vec<String> = Preset::Quality
            .overrides()
            .into_iter()
            .chain(Preset::LowVram.overrides())
            .chain(Preset::MidVram.overrides())
            .map(|(k, _)| k)
            .collect();
        for name in ENV_NAMES
            .iter()
            .copied()
            .chain(preset_names.iter().map(String::as_str))
        {
            assert!(
                sources.contains(&format!("\"{name}\"")),
                "{name} is read nowhere in third_party/victory"
            );
        }
    }

    fn collect_rs(dir: &Path, into: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                into.push_str(&text);
            }
        }
    }
}
