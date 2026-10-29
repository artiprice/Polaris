// Unit tests of apps/polaris-cli/src/cli_settings.rs (`tests`); compiled into that module via #[path].
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
fn settings_worker_cancellation_discards_stale_generation() -> Result<(), Box<dyn std::error::Error>>
{
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
