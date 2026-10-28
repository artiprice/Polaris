//! Saved UI choices are separate from transient launch overrides and terminal capabilities.
use crate::{theme::ThemeMode, usage::UsageBarStyle};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorPolicy {
    #[default]
    Auto,
    Always,
    Never,
}
impl ColorPolicy {
    pub fn enabled(self, auto: bool) -> bool {
        match self {
            Self::Auto => auto,
            Self::Always => true,
            Self::Never => false,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Always => "Always",
            Self::Never => "Never",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub schema_version: u32,
    pub usage_bar_style: UsageBarStyle,
    pub stats_chart_style: crate::stats::ChartStyle,
    pub theme: ThemeMode,
    pub color: ColorPolicy,
    pub ascii: bool,
    pub reduced_motion: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            schema_version: 1,
            usage_bar_style: UsageBarStyle::ContinuousStrip,
            stats_chart_style: crate::stats::ChartStyle::Stacked,
            theme: ThemeMode::Dark,
            color: ColorPolicy::Auto,
            ascii: false,
            reduced_motion: false,
            extra: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreferenceField {
    UsageBar,
    StatsChart,
    Theme,
    Color,
    Ascii,
    Motion,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Session,
    Defaults,
    Saved,
    Warning(String),
    Error(String),
}
impl SaveStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Session => "Session preview",
            Self::Defaults => "Defaults / autosave",
            Self::Saved => "Saved",
            Self::Warning(_) => "Load warning",
            Self::Error(_) => "Save failed",
        }
    }
}
#[derive(Clone, Debug)]
pub struct PreferenceState {
    pub values: UiPreferences,
    pub saved: UiPreferences,
    pub overrides: BTreeSet<PreferenceField>,
    pub path: Option<PathBuf>,
    pub status: SaveStatus,
    pub auto_color: bool,
}
impl Default for PreferenceState {
    fn default() -> Self {
        Self {
            values: UiPreferences::default(),
            saved: UiPreferences::default(),
            overrides: BTreeSet::new(),
            path: None,
            status: SaveStatus::Session,
            auto_color: true,
        }
    }
}
impl PreferenceState {
    pub fn load(path: PathBuf, auto_color: bool) -> Self {
        let mut state = Self {
            path: Some(path.clone()),
            auto_color,
            ..Self::default()
        };
        match fs::read(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => state.status = SaveStatus::Defaults,
            Err(e) => {
                state.status = SaveStatus::Warning(format!("Cannot read UI preferences: {e}"))
            }
            Ok(bytes) => {
                match serde_json::from_slice::<UiPreferences>(&bytes) {
                    Ok(p) if p.schema_version == 1 => {
                        state.values = p.clone();
                        state.saved = p;
                        state.status = SaveStatus::Saved;
                    }
                    Ok(_) => state.status = SaveStatus::Warning(
                        "Unsupported UI preference version; defaults used. File kept unchanged."
                            .into(),
                    ),
                    Err(e) => {
                        state.status = SaveStatus::Warning(format!(
                            "Invalid UI preferences; defaults used. File kept unchanged: {e}"
                        ))
                    }
                }
            }
        }
        state
    }
    /// Copy only the field the user explicitly changed. Other effective values may be CLI overrides.
    pub fn commit(&mut self, field: PreferenceField) {
        match field {
            PreferenceField::UsageBar => self.saved.usage_bar_style = self.values.usage_bar_style,
            PreferenceField::StatsChart => {
                self.saved.stats_chart_style = self.values.stats_chart_style
            }
            PreferenceField::Theme => self.saved.theme = self.values.theme,
            PreferenceField::Color => self.saved.color = self.values.color,
            PreferenceField::Ascii => self.saved.ascii = self.values.ascii,
            PreferenceField::Motion => self.saved.reduced_motion = self.values.reduced_motion,
        }
        self.overrides.remove(&field);
        let Some(path) = &self.path else {
            return;
        };
        self.status = match atomic_save(path, &self.saved) {
            Ok(()) => SaveStatus::Saved,
            Err(e) => SaveStatus::Error(format!(
                "Could not save UI preferences: {e}. Session changes remain active."
            )),
        };
    }
}
pub fn configuration_path() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("POLARIS_CONFIG_DIR").filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .filter(|s| !s.is_empty())
            .map(|p| PathBuf::from(p).join("Polaris/settings.json"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .map(|p| p.join("polaris/settings.json"))
    }
}
static SAVE_ID: AtomicU64 = AtomicU64::new(0);
pub fn atomic_save(path: &Path, preferences: &UiPreferences) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".settings-{}-{}.tmp",
        std::process::id(),
        SAVE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        let bytes = serde_json::to_vec_pretty(preferences).map_err(io::Error::other)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}
#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::winbase::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both paths are valid, NUL-terminated UTF-16 buffers alive for the call.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
