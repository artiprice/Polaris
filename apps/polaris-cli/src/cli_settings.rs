//! Cold settings-screen state and explicit global preference persistence.
//!
//! Runtime, session, and demonstration overrides are kept in memory until the caller explicitly
//! invokes [`CliSettings::apply_selection`].

use polaris_config::{
    CliColorDepth, CliColorPolicy, CliPanelDefault, CliPreferences, CliTheme, ConfigError, Dirs,
    PreferenceField, PreferenceValue, read_global_cli_preferences, save_global_cli_preference,
};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::ui_services::{PreferenceOverrides, ServiceError};

const SETTINGS_QUEUE_CAPACITY: usize = 1;
const SETTINGS_SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// Setting currently selected in the settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingField {
    /// CLI canvas theme.
    Theme,
    /// CLI color policy.
    Color,
    /// CLI palette depth.
    ColorDepth,
    /// ASCII drawing mode.
    Ascii,
    /// Reduced motion preference.
    ReduceMotion,
    /// Initial side panel visibility.
    Panel,
}

/// Navigation direction for the settings selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    /// Select the next field.
    Next,
    /// Select the previous field.
    Previous,
}

/// Whether the helper may persist a selection to the global config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistencePolicy {
    /// Explicit selection may update `Dirs.global/config.toml`.
    Global,
    /// Memory-only mode for render fixtures and demo sessions.
    MemoryOnly,
}

/// Render-ready settings row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    /// Preference field represented by this row.
    pub field: SettingField,
    /// Human-readable row title.
    pub label: &'static str,
    /// Current effective value.
    pub value: &'static str,
    /// Whether this row is selected.
    pub selected: bool,
}

/// Owns persisted global preferences and nonpersistent runtime overrides for the CLI settings UI.
#[derive(Debug, Clone)]
pub struct CliSettings {
    dirs: Dirs,
    persisted: CliPreferences,
    overrides: PreferenceOverrides,
    edited_fields: u8,
    selected: SettingField,
    persistence: PersistencePolicy,
}

impl CliSettings {
    /// Load global preferences and retain runtime/session overrides in memory.
    ///
    /// # Errors
    /// Returns an I/O, size, parse, or preference validation error from the global config file.
    pub fn load(dirs: Dirs, overrides: PreferenceOverrides) -> Result<Self, ConfigError> {
        let persisted = read_global_cli_preferences(&dirs)?;
        Ok(Self::from_snapshot(
            dirs,
            persisted,
            overrides,
            PersistencePolicy::Global,
        ))
    }

    /// Construct from already layered effective preferences without reading config again.
    ///
    /// Pass [`PersistencePolicy::MemoryOnly`] for render fixtures and demo sessions. In that mode,
    /// [`CliSettings::apply_selection`] is rejected so a fixture cannot alter user configuration.
    #[must_use]
    pub fn from_snapshot(
        dirs: Dirs,
        preferences: CliPreferences,
        overrides: PreferenceOverrides,
        persistence: PersistencePolicy,
    ) -> Self {
        Self {
            dirs,
            persisted: preferences,
            overrides,
            edited_fields: 0,
            selected: SettingField::Theme,
            persistence,
        }
    }

    /// Change whether explicit saves may write the global config.
    pub fn set_persistence(&mut self, persistence: PersistencePolicy) {
        self.persistence = persistence;
    }

    /// Current persistence policy.
    #[must_use]
    pub const fn persistence(&self) -> PersistencePolicy {
        self.persistence
    }

    /// Effective preferences after memory-only runtime/session overrides.
    #[must_use]
    pub fn effective(&self) -> CliPreferences {
        self.overrides.apply(self.persisted)
    }

    /// Apply a whitelisted demo preference in memory; refuses a persistent settings owner.
    pub fn apply_demo_choice(&mut self, key: &str, value: &str) -> bool {
        if self.persistence != PersistencePolicy::MemoryOnly {
            return false;
        }
        match (key, value) {
            ("theme", "dark") => self.overrides.theme = Some(CliTheme::Dark),
            ("theme", "terminal") => self.overrides.theme = Some(CliTheme::Terminal),
            ("color", "auto") => self.overrides.color = Some(CliColorPolicy::Auto),
            ("color", "always") => self.overrides.color = Some(CliColorPolicy::Always),
            ("color", "never") => self.overrides.color = Some(CliColorPolicy::Never),
            ("ascii", "true") => self.overrides.ascii = Some(true),
            ("ascii", "false") => self.overrides.ascii = Some(false),
            ("reduce_motion", "true") => self.overrides.reduce_motion = Some(true),
            ("reduce_motion", "false") => self.overrides.reduce_motion = Some(false),
            _ => return false,
        }
        true
    }

    /// Selected settings field.
    #[must_use]
    pub const fn selected(&self) -> SettingField {
        self.selected
    }

    /// Whether the selected preference has an explicit settings-screen edit pending.
    #[must_use]
    pub fn has_pending_edit(&self) -> bool {
        self.edited_fields & field_bit(self.selected) != 0
    }

    /// Select the next or previous row in the stable settings order.
    pub fn navigate(&mut self, direction: Navigation) {
        self.selected = match (self.selected, direction) {
            (SettingField::Theme, Navigation::Next) => SettingField::Color,
            (SettingField::Color, Navigation::Next) => SettingField::ColorDepth,
            (SettingField::ColorDepth, Navigation::Next) => SettingField::Ascii,
            (SettingField::Ascii, Navigation::Next) => SettingField::ReduceMotion,
            (SettingField::ReduceMotion, Navigation::Next) => SettingField::Panel,
            (SettingField::Panel, Navigation::Next) => SettingField::Theme,
            (SettingField::Theme, Navigation::Previous) => SettingField::Panel,
            (SettingField::Color, Navigation::Previous) => SettingField::Theme,
            (SettingField::ColorDepth, Navigation::Previous) => SettingField::Color,
            (SettingField::Ascii, Navigation::Previous) => SettingField::ColorDepth,
            (SettingField::ReduceMotion, Navigation::Previous) => SettingField::Ascii,
            (SettingField::Panel, Navigation::Previous) => SettingField::ReduceMotion,
        };
    }

    /// Cycle the selected field and update only the memory-only override.
    pub fn cycle(&mut self) {
        let current = self.effective();
        self.edited_fields |= field_bit(self.selected);
        match self.selected {
            SettingField::Theme => {
                self.overrides.theme = Some(match current.theme {
                    CliTheme::Dark => CliTheme::Terminal,
                    CliTheme::Terminal => CliTheme::Dark,
                });
            }
            SettingField::Color => {
                self.overrides.color = Some(match current.color {
                    CliColorPolicy::Auto => CliColorPolicy::Always,
                    CliColorPolicy::Always => CliColorPolicy::Never,
                    CliColorPolicy::Never => CliColorPolicy::Auto,
                });
            }
            SettingField::ColorDepth => {
                self.overrides.color_depth = Some(match current.color_depth {
                    CliColorDepth::Auto => CliColorDepth::Truecolor,
                    CliColorDepth::Truecolor => CliColorDepth::Ansi,
                    CliColorDepth::Ansi => CliColorDepth::Auto,
                });
            }
            SettingField::Ascii => self.overrides.ascii = Some(!current.ascii),
            SettingField::ReduceMotion => {
                self.overrides.reduce_motion = Some(!current.reduce_motion)
            }
            SettingField::Panel => {
                self.overrides.panel = Some(match current.panel {
                    CliPanelDefault::Open => CliPanelDefault::Closed,
                    CliPanelDefault::Closed => CliPanelDefault::Open,
                });
            }
        }
    }

    /// Current rows in display order, using effective values after runtime overrides.
    #[must_use]
    pub fn rows(&self) -> [SettingRow; 6] {
        let preferences = self.effective();
        [
            SettingRow {
                field: SettingField::Theme,
                label: "Theme",
                value: theme_name(preferences.theme),
                selected: self.selected == SettingField::Theme,
            },
            SettingRow {
                field: SettingField::Color,
                label: "Color",
                value: color_name(preferences.color),
                selected: self.selected == SettingField::Color,
            },
            SettingRow {
                field: SettingField::ColorDepth,
                label: "Color depth",
                value: color_depth_name(preferences.color_depth),
                selected: self.selected == SettingField::ColorDepth,
            },
            SettingRow {
                field: SettingField::Ascii,
                label: "ASCII decoration",
                value: boolean_name(preferences.ascii),
                selected: self.selected == SettingField::Ascii,
            },
            SettingRow {
                field: SettingField::ReduceMotion,
                label: "Reduce motion",
                value: boolean_name(preferences.reduce_motion),
                selected: self.selected == SettingField::ReduceMotion,
            },
            SettingRow {
                field: SettingField::Panel,
                label: "Default panel",
                value: panel_name(preferences.panel),
                selected: self.selected == SettingField::Panel,
            },
        ]
    }

    /// Persist the currently selected value to `dirs.global/config.toml` if it was explicitly
    /// edited in the settings screen.
    ///
    /// This is the explicit save boundary. Calling [`CliSettings::cycle`] alone never writes any
    /// configuration file, so demo/session callers can use the same state only in memory.
    ///
    /// # Errors
    /// Returns an I/O, size, parse, encoding, or field/value mismatch error. On failure the saved
    /// preferences in this object remain unchanged.
    pub fn apply_selection(&mut self) -> Result<CliPreferences, ConfigError> {
        if self.persistence == PersistencePolicy::MemoryOnly {
            return Err(ConfigError::Invalid {
                key: "ui.cli".to_owned(),
                reason: "persistence is disabled for this settings session".to_owned(),
            });
        }
        if !self.has_pending_edit() {
            return Ok(self.effective());
        }
        let value = preference_value(self.selected, self.effective());
        save_global_cli_preference(&self.dirs, config_field(self.selected), value)?;
        self.persisted = replace_field(self.persisted, self.selected, self.effective());
        clear_override(&mut self.overrides, self.selected);
        self.edited_fields &= !field_bit(self.selected);
        Ok(self.effective())
    }
}

const fn field_bit(field: SettingField) -> u8 {
    match field {
        SettingField::Theme => 1,
        SettingField::Color => 2,
        SettingField::ColorDepth => 4,
        SettingField::Ascii => 8,
        SettingField::ReduceMotion => 16,
        SettingField::Panel => 32,
    }
}

/// Completion of an off-thread settings save.
#[derive(Debug)]
pub struct SettingsResult {
    /// UI generation that queued the save.
    pub generation: u64,
    /// Updated settings state, or a generic user-facing failure.
    pub result: Result<CliSettings, String>,
}

enum SettingsCommand {
    Save {
        generation: u64,
        settings: CliSettings,
    },
}

/// Bounded worker that performs explicit settings persistence away from the UI thread.
///
/// Results from retired generations are discarded. Shutdown waits up to two seconds for the
/// worker to finish and joins it if it has exited; a slow filesystem operation may finish after
/// shutdown returns, but its write uses the config owner's atomic-save path.
pub struct SettingsService {
    requests: Option<SyncSender<SettingsCommand>>,
    results: Receiver<SettingsResult>,
    active_generation: Arc<Mutex<u64>>,
    worker: Option<JoinHandle<()>>,
    finished: Receiver<()>,
}

impl SettingsService {
    /// Start the settings persistence worker.
    ///
    /// # Errors
    /// Returns [`ServiceError::Unavailable`] if the worker thread cannot be created.
    pub fn new() -> Result<Self, ServiceError> {
        let (request_tx, request_rx) = mpsc::sync_channel(SETTINGS_QUEUE_CAPACITY);
        let (result_tx, result_rx) = mpsc::sync_channel(SETTINGS_QUEUE_CAPACITY);
        let (finished_tx, finished_rx) = mpsc::sync_channel(1);
        let active_generation = Arc::new(Mutex::new(0_u64));
        let worker_generation = Arc::clone(&active_generation);
        let worker = thread::Builder::new()
            .name("polaris-cli-settings".to_owned())
            .spawn(move || {
                settings_worker(request_rx, result_tx, worker_generation);
                let _ = finished_tx.try_send(());
            })
            .map_err(|_| ServiceError::Unavailable)?;
        Ok(Self {
            requests: Some(request_tx),
            results: result_rx,
            active_generation,
            worker: Some(worker),
            finished: finished_rx,
        })
    }

    /// Queue a save without blocking the caller on filesystem work.
    ///
    /// # Errors
    /// Returns [`ServiceError::QueueFull`] if the one-item queue is occupied or closed.
    pub fn request_save(
        &mut self,
        generation: u64,
        settings: CliSettings,
    ) -> Result<(), ServiceError> {
        let Some(requests) = self.requests.as_ref() else {
            return Err(ServiceError::QueueFull);
        };
        let mut active_generation = lock_generation(&self.active_generation);
        let previous_generation = *active_generation;
        *active_generation = generation;
        match requests.try_send(SettingsCommand::Save {
            generation,
            settings,
        }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                *active_generation = previous_generation;
                Err(ServiceError::QueueFull)
            }
        }
    }

    /// Retire results and queued work for `generation`.
    pub fn cancel(&mut self, generation: u64) {
        let mut active = lock_generation(&self.active_generation);
        if *active == generation {
            *active = 0;
        }
    }

    /// Return the next nonblocking result from the active UI generation.
    #[must_use]
    pub fn try_recv(&self) -> Option<SettingsResult> {
        loop {
            match self.results.try_recv() {
                Ok(result) if self.is_current(result.generation) => return Some(result),
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Close the queue and wait briefly for the worker to exit.
    pub fn shutdown(&mut self) {
        *lock_generation(&self.active_generation) = 0;
        self.requests.take();
        if let Some(worker) = self.worker.take()
            && matches!(self.finished.recv_timeout(SETTINGS_SHUTDOWN_WAIT), Ok(()))
        {
            let _ = worker.join();
        }
    }

    fn is_current(&self, generation: u64) -> bool {
        *lock_generation(&self.active_generation) == generation
    }
}

fn lock_generation(generation: &Mutex<u64>) -> std::sync::MutexGuard<'_, u64> {
    match generation.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn settings_worker(
    requests: Receiver<SettingsCommand>,
    results: SyncSender<SettingsResult>,
    active_generation: Arc<Mutex<u64>>,
) {
    loop {
        let command = match requests.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let SettingsCommand::Save {
            generation,
            mut settings,
        } = command;
        if *lock_generation(&active_generation) != generation {
            continue;
        }
        let result = settings
            .apply_selection()
            .map(|_| settings)
            .map_err(|_| "Could not save CLI preferences.".to_owned());
        if *lock_generation(&active_generation) == generation
            && matches!(
                results.try_send(SettingsResult { generation, result }),
                Ok(())
            )
        {
            continue;
        }
    }
}

fn preference_value(field: SettingField, value: CliPreferences) -> PreferenceValue {
    match field {
        SettingField::Theme => PreferenceValue::Theme(value.theme),
        SettingField::Color => PreferenceValue::Color(value.color),
        SettingField::ColorDepth => PreferenceValue::ColorDepth(value.color_depth),
        SettingField::Ascii | SettingField::ReduceMotion => {
            let boolean = match field {
                SettingField::Ascii => value.ascii,
                SettingField::ReduceMotion => value.reduce_motion,
                SettingField::Theme
                | SettingField::Color
                | SettingField::ColorDepth
                | SettingField::Panel => false,
            };
            PreferenceValue::Boolean(boolean)
        }
        SettingField::Panel => PreferenceValue::Panel(value.panel),
    }
}

fn config_field(field: SettingField) -> PreferenceField {
    match field {
        SettingField::Theme => PreferenceField::Theme,
        SettingField::Color => PreferenceField::Color,
        SettingField::ColorDepth => PreferenceField::ColorDepth,
        SettingField::Ascii => PreferenceField::Ascii,
        SettingField::ReduceMotion => PreferenceField::ReduceMotion,
        SettingField::Panel => PreferenceField::Panel,
    }
}

fn replace_field(
    mut preferences: CliPreferences,
    field: SettingField,
    value: CliPreferences,
) -> CliPreferences {
    match field {
        SettingField::Theme => preferences.theme = value.theme,
        SettingField::Color => preferences.color = value.color,
        SettingField::ColorDepth => preferences.color_depth = value.color_depth,
        SettingField::Ascii => preferences.ascii = value.ascii,
        SettingField::ReduceMotion => preferences.reduce_motion = value.reduce_motion,
        SettingField::Panel => preferences.panel = value.panel,
    }
    preferences
}

fn clear_override(overrides: &mut PreferenceOverrides, field: SettingField) {
    match field {
        SettingField::Theme => overrides.theme = None,
        SettingField::Color => overrides.color = None,
        SettingField::ColorDepth => overrides.color_depth = None,
        SettingField::Ascii => overrides.ascii = None,
        SettingField::ReduceMotion => overrides.reduce_motion = None,
        SettingField::Panel => overrides.panel = None,
    }
}

fn theme_name(theme: CliTheme) -> &'static str {
    match theme {
        CliTheme::Dark => "dark",
        CliTheme::Terminal => "terminal",
    }
}

fn color_name(color: CliColorPolicy) -> &'static str {
    match color {
        CliColorPolicy::Auto => "auto",
        CliColorPolicy::Always => "always",
        CliColorPolicy::Never => "never",
    }
}

fn color_depth_name(depth: CliColorDepth) -> &'static str {
    match depth {
        CliColorDepth::Auto => "auto",
        CliColorDepth::Truecolor => "truecolor",
        CliColorDepth::Ansi => "ansi",
    }
}

fn boolean_name(value: bool) -> &'static str {
    match value {
        true => "on",
        false => "off",
    }
}

fn panel_name(panel: CliPanelDefault) -> &'static str {
    match panel {
        CliPanelDefault::Open => "open",
        CliPanelDefault::Closed => "closed",
    }
}

#[cfg(test)]
mod tests {
    use super::{CliSettings, Navigation, PersistencePolicy, SettingField, SettingsService};
    use crate::ui_services::PreferenceOverrides;
    use polaris_config::{CliPanelDefault, Dirs};
    use std::{fs, thread, time::Duration};

    #[test]
    fn cycle_is_memory_only_until_explicit_apply() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: Some(directory.path().join(".polaris")),
        };
        let mut settings = CliSettings::load(dirs, PreferenceOverrides::default())?;
        settings.cycle();
        assert_eq!(
            settings.effective().theme,
            polaris_config::CliTheme::Terminal
        );
        assert!(!directory.path().join("config.toml").exists());
        settings.apply_selection()?;
        let saved = fs::read_to_string(directory.path().join("config.toml"))?;
        assert!(saved.contains("theme = \"terminal\""));
        assert_eq!(
            settings.effective().theme,
            polaris_config::CliTheme::Terminal
        );
        Ok(())
    }

    #[test]
    fn navigation_and_rows_are_stable() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let mut settings = CliSettings::load(dirs, PreferenceOverrides::default())?;
        assert_eq!(settings.rows().len(), 6);
        assert_eq!(settings.selected(), SettingField::Theme);
        settings.navigate(Navigation::Previous);
        assert_eq!(settings.selected(), SettingField::Panel);
        settings.cycle();
        assert_eq!(settings.effective().panel, CliPanelDefault::Open);
        assert!(
            settings
                .rows()
                .iter()
                .any(|row| row.selected && row.field == SettingField::Panel)
        );
        Ok(())
    }

    #[test]
    fn untouched_runtime_overrides_are_not_persisted() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let overrides = PreferenceOverrides {
            theme: Some(polaris_config::CliTheme::Terminal),
            ascii: Some(true),
            ..PreferenceOverrides::default()
        };
        let mut settings = CliSettings::load(dirs, overrides)?;
        assert!(!settings.has_pending_edit());
        settings.apply_selection()?;
        assert!(!directory.path().join("config.toml").exists());
        assert_eq!(
            settings.effective().theme,
            polaris_config::CliTheme::Terminal
        );
        assert!(settings.effective().ascii);
        Ok(())
    }

    #[test]
    fn explicit_edit_persists_only_edited_field_and_keeps_other_overrides()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let overrides = PreferenceOverrides {
            theme: Some(polaris_config::CliTheme::Terminal),
            ascii: Some(true),
            ..PreferenceOverrides::default()
        };
        let mut settings = CliSettings::load(dirs, overrides)?;
        assert!(!settings.has_pending_edit());
        settings.cycle();
        assert!(settings.has_pending_edit());
        settings.apply_selection()?;
        let saved = fs::read_to_string(directory.path().join("config.toml"))?;
        assert!(saved.contains("theme = \"dark\""));
        assert!(!saved.contains("ascii ="));
        assert_eq!(settings.effective().theme, polaris_config::CliTheme::Dark);
        assert!(settings.effective().ascii);
        Ok(())
    }

    #[test]
    fn memory_only_settings_reject_persistence() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let mut settings = CliSettings::from_snapshot(
            dirs,
            polaris_config::CliPreferences::default(),
            PreferenceOverrides::default(),
            PersistencePolicy::MemoryOnly,
        );
        settings.cycle();
        assert!(settings.apply_selection().is_err());
        assert!(!directory.path().join("config.toml").exists());
        Ok(())
    }

    #[test]
    fn worker_persists_on_a_worker_and_returns_updated_settings()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let mut settings = CliSettings::from_snapshot(
            dirs,
            polaris_config::CliPreferences::default(),
            PreferenceOverrides::default(),
            PersistencePolicy::Global,
        );
        settings.cycle();
        let mut service = SettingsService::new()?;
        service.request_save(7, settings)?;
        let result = receive_settings(&service)?;
        assert_eq!(result.generation, 7);
        assert!(result.result.is_ok());
        assert!(directory.path().join("config.toml").exists());
        service.shutdown();
        Ok(())
    }

    #[test]
    fn memory_only_worker_save_does_not_write_and_cancel_discards_result()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let settings = CliSettings::from_snapshot(
            dirs,
            polaris_config::CliPreferences::default(),
            PreferenceOverrides::default(),
            PersistencePolicy::MemoryOnly,
        );
        let mut service = SettingsService::new()?;
        service.request_save(19, settings)?;
        let result = receive_settings(&service)?;
        assert_eq!(result.generation, 19);
        assert!(result.result.is_err());
        assert!(!directory.path().join("config.toml").exists());
        service.cancel(19);
        assert!(service.try_recv().is_none());
        service.shutdown();
        Ok(())
    }

    #[test]
    fn settings_worker_cancellation_discards_stale_generation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let dirs = Dirs {
            global: directory.path().to_path_buf(),
            repo: None,
        };
        let settings = CliSettings::from_snapshot(
            dirs,
            polaris_config::CliPreferences::default(),
            PreferenceOverrides::default(),
            PersistencePolicy::MemoryOnly,
        );
        let mut service = SettingsService::new()?;
        service.request_save(23, settings)?;
        service.cancel(23);
        thread::sleep(Duration::from_millis(20));
        assert!(service.try_recv().is_none());
        assert!(!directory.path().join("config.toml").exists());
        service.shutdown();
        Ok(())
    }

    fn receive_settings(
        service: &SettingsService,
    ) -> Result<super::SettingsResult, Box<dyn std::error::Error>> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = service.try_recv() {
                return Ok(result);
            }
            if std::time::Instant::now() >= deadline {
                return Err("settings worker result timed out".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}
