use polaris::{
    app::App,
    preferences::{
        ColorPolicy, PreferenceField, PreferenceState, SaveStatus, UiPreferences, atomic_save,
    },
    settings::{self, SettingsAction},
    theme::ThemeMode,
    usage::UsageBarStyle,
};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
fn folder() -> PathBuf {
    let p = std::env::current_dir()
        .unwrap()
        .join("target/settings-tests")
        .join(format!(
            "{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn choose(app: &mut App, category: usize, row: usize, value: usize) {
    settings::activate(app, SettingsAction::Category(category));
    settings::activate(app, SettingsAction::Choose { row, value });
    settings::activate(app, SettingsAction::Apply);
}
#[test]
fn navigation_preserves_file_effective_values_saved_state_and_overrides_until_enter() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let path = folder().join("settings.json");
    atomic_save(&path, &UiPreferences::default()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let mut app = App::default();
    app.command("/settings");
    app.preferences = PreferenceState::load(path.clone(), true);
    app.preferences.values.ascii = true;
    app.preferences.overrides.insert(PreferenceField::Ascii);
    settings::sync_effective(&mut app);
    let values = app.preferences.values.clone();
    let saved = app.preferences.saved.clone();
    let overrides = app.preferences.overrides.clone();
    for (category, row, value) in [
        (0, 0, 1),
        (0, 1, 0),
        (1, 2, 0),
        (3, 0, 1),
        (4, 0, 2),
        (4, 1, 1),
        (5, 0, 0),
    ] {
        settings::activate(&mut app, SettingsAction::Category(category));
        settings::activate(&mut app, SettingsAction::Choose { row, value });
        settings::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
        );
        assert_eq!(app.preferences.values, values);
        assert_eq!(app.preferences.saved, saved);
        assert_eq!(app.preferences.overrides, overrides);
        assert_eq!(app.theme.mode, ThemeMode::Dark);
        assert!(app.theme.color && app.ascii && !app.reduce_motion);
        assert_eq!(app.mode, 0);
        assert_eq!(app.screen, polaris::app::Screen::Settings);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    settings::activate(
        &mut app,
        SettingsAction::Category(settings::SettingsCategory::Accessibility.index()),
    );
    settings::activate(&mut app, SettingsAction::Select(0));
    settings::handle_key(&mut app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    settings::activate(&mut app, SettingsAction::Apply);
    let restart = PreferenceState::load(path.clone(), true);
    let expected = UiPreferences {
        color: ColorPolicy::Always,
        ..UiPreferences::default()
    };
    assert_eq!(restart.values, expected);
    assert!(app.preferences.overrides.contains(&PreferenceField::Ascii));
    settings::activate(&mut app, SettingsAction::Choose { row: 1, value: 1 });
    app.close();
    assert!(!PreferenceState::load(path, true).values.reduced_motion);
}
#[test]
fn reconfirm_is_noop_except_launch_override_or_failed_save_retry() {
    let path = folder().join("settings.json");
    // Deliberate formatting proves a no-op does not rewrite even identical JSON values.
    let bytes = b"{\"ascii\":false}\n";
    fs::write(&path, bytes).unwrap();
    let mut app = App::default();
    app.preferences = PreferenceState::load(path.clone(), true);
    choose(&mut app, 0, 1, 0);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    app.preferences.overrides.insert(PreferenceField::Ascii);
    choose(&mut app, 0, 1, 0);
    assert!(!app.preferences.overrides.contains(&PreferenceField::Ascii));
    assert_ne!(fs::read(&path).unwrap(), bytes);
    let blocked = folder().join("blocked");
    fs::write(&blocked, "obstruction").unwrap();
    app.preferences = PreferenceState::load(blocked.join("settings.json"), true);
    choose(&mut app, 0, 1, 1);
    assert!(matches!(app.preferences.status, SaveStatus::Error(_)));
    settings::activate(&mut app, SettingsAction::Choose { row: 1, value: 0 });
    assert!(matches!(app.preferences.status, SaveStatus::Error(_)));
    // Restore the confirmed candidate, repair the isolated store, and retry unchanged value.
    settings::activate(&mut app, SettingsAction::Select(1));
    fs::remove_file(&blocked).unwrap();
    settings::activate(&mut app, SettingsAction::Apply);
    assert_eq!(app.preferences.status, SaveStatus::Saved);
    assert!(
        PreferenceState::load(blocked.join("settings.json"), true)
            .values
            .ascii
    );
}
#[test]
fn missing_file_uses_strip_defaults_and_loading_does_not_write() {
    let path = folder().join("nested/settings.json");
    let p = PreferenceState::load(path.clone(), false);
    assert_eq!(p.values, UiPreferences::default());
    assert_eq!(p.values.usage_bar_style, UsageBarStyle::ContinuousStrip);
    assert_eq!(UsageBarStyle::default(), UsageBarStyle::ContinuousStrip);
    assert_eq!(p.status, SaveStatus::Defaults);
    assert!(!p.values.color.enabled(p.auto_color));
    assert!(!path.exists());
}
#[test]
fn every_persistent_control_applies_on_confirmation_and_survives_restart() {
    let path = folder().join("settings.json");
    let mut app = App::default();
    app.preferences = PreferenceState::load(path.clone(), true);
    choose(&mut app, 0, 0, 1);
    assert_eq!(app.theme.mode, ThemeMode::Terminal);
    choose(&mut app, 0, 1, 1);
    assert!(app.ascii);
    choose(
        &mut app,
        settings::SettingsCategory::Accessibility.index(),
        0,
        2,
    );
    assert!(!app.theme.color);
    choose(
        &mut app,
        settings::SettingsCategory::Accessibility.index(),
        1,
        1,
    );
    assert!(app.reduce_motion);
    for (row, style) in [
        (0, UsageBarStyle::ContinuousStrip),
        (1, UsageBarStyle::FineTicks),
        (2, UsageBarStyle::SegmentedRail),
    ] {
        choose(&mut app, 1, row, 0);
        assert_eq!(app.preferences.status, SaveStatus::Saved);
        let restart = PreferenceState::load(path.clone(), true);
        assert_eq!(restart.values.usage_bar_style, style);
        assert_eq!(restart.values.theme, ThemeMode::Terminal);
        assert_eq!(restart.values.color, ColorPolicy::Never);
        assert!(restart.values.ascii && restart.values.reduced_motion);
    }
    assert!(
        fs::read_dir(path.parent().unwrap()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp"))
    );
}
#[test]
fn saving_a_field_does_not_save_unrelated_launch_overrides() {
    let path = folder().join("settings.json");
    let base = UiPreferences::default();
    atomic_save(&path, &base).unwrap();
    let mut app = App::default();
    app.preferences = PreferenceState::load(path.clone(), true);
    app.preferences.values.theme = ThemeMode::Terminal;
    app.preferences.values.ascii = true;
    app.preferences.values.color = ColorPolicy::Never;
    app.preferences.overrides.extend([
        PreferenceField::Theme,
        PreferenceField::Ascii,
        PreferenceField::Color,
    ]);
    settings::sync_effective(&mut app);
    choose(&mut app, 1, 2, 0);
    let saved = PreferenceState::load(path.clone(), true);
    assert_eq!(saved.values.theme, ThemeMode::Dark);
    assert!(!saved.values.ascii);
    assert_eq!(saved.values.color, ColorPolicy::Auto);
    assert_eq!(app.theme.mode, ThemeMode::Terminal);
    assert!(app.ascii);
    assert!(!app.theme.color);
    assert!(app.preferences.overrides.contains(&PreferenceField::Theme));
    choose(&mut app, 0, 0, 1);
    assert!(!app.preferences.overrides.contains(&PreferenceField::Theme));
    assert_eq!(
        PreferenceState::load(path, true).values.theme,
        ThemeMode::Terminal
    );
}
#[test]
fn interaction_and_providers_are_session_only_and_never_create_preferences() {
    let path = folder().join("settings.json");
    let mut app = App::default();
    app.preferences = PreferenceState::load(path.clone(), true);
    choose(
        &mut app,
        settings::SettingsCategory::Interaction.index(),
        0,
        1,
    );
    assert_eq!(app.mode, 1);
    assert!(!path.exists());
    choose(
        &mut app,
        settings::SettingsCategory::Providers.index(),
        0,
        0,
    );
    assert_eq!(app.screen, polaris::app::Screen::Providers);
    assert!(!path.exists());
    assert_eq!(App::default().mode, 0);
}
#[test]
fn invalid_and_future_files_warn_without_overwriting_until_explicit_change() {
    for content in [
        "{bad json",
        r#"{"schema_version":99,"usage_bar_style":"strip"}"#,
        r#"{"color":"invalid"}"#,
        r#"{"ascii":"yes"}"#,
    ] {
        let path = folder().join("settings.json");
        fs::write(&path, content).unwrap();
        let mut p = PreferenceState::load(path.clone(), true);
        assert!(matches!(p.status, SaveStatus::Warning(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
        assert_eq!(p.values.usage_bar_style, UsageBarStyle::ContinuousStrip);
        p.values.usage_bar_style = UsageBarStyle::FineTicks;
        p.commit(PreferenceField::UsageBar);
        assert_eq!(p.status, SaveStatus::Saved);
        assert_eq!(
            PreferenceState::load(path, true).values.usage_bar_style,
            UsageBarStyle::FineTicks
        );
    }
}
#[test]
fn unknown_fields_survive_roundtrip_and_partial_files_take_defaults() {
    let path = folder().join("settings.json");
    fs::write(&path, r#"{"ascii":true,"future_option":{"key":7}}"#).unwrap();
    let mut p = PreferenceState::load(path.clone(), true);
    assert_eq!(p.values.usage_bar_style, UsageBarStyle::ContinuousStrip);
    assert!(p.values.ascii);
    p.values.reduced_motion = true;
    p.commit(PreferenceField::Motion);
    let raw: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(raw["future_option"]["key"], 7);
}
#[test]
fn failed_saves_leave_session_choices_active_and_report_error() {
    let parent = folder().join("blocked");
    fs::write(&parent, "keep this file").unwrap();
    let mut app = App::default();
    app.preferences = PreferenceState::load(parent.join("settings.json"), true);
    choose(&mut app, 1, 2, 0);
    assert!(matches!(app.preferences.status, SaveStatus::Error(_)));
    assert_eq!(
        app.preferences.values.usage_bar_style,
        UsageBarStyle::SegmentedRail
    );
    assert_eq!(fs::read_to_string(parent).unwrap(), "keep this file");
}
#[cfg(windows)]
#[test]
fn failed_atomic_replacement_preserves_the_last_good_file_and_cleans_temp() {
    use std::os::windows::fs::OpenOptionsExt;
    let path = folder().join("settings.json");
    let original = UiPreferences::default();
    atomic_save(&path, &original).unwrap();
    let bytes = fs::read(&path).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let changed = UiPreferences {
        ascii: true,
        ..original
    };
    assert!(atomic_save(&path, &changed).is_err());
    drop(lock);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}
#[test]
fn auto_policy_and_explicit_policies_respect_environment_resolution() {
    for auto in [false, true] {
        assert_eq!(ColorPolicy::Auto.enabled(auto), auto);
        assert!(ColorPolicy::Always.enabled(auto));
        assert!(!ColorPolicy::Never.enabled(auto));
    }
}
#[test]
fn command_launch_loads_preferences_across_workspaces_and_flags_do_not_write() {
    let dir = folder();
    let config = dir.join("config");
    fs::create_dir_all(&config).unwrap();
    let path = config.join("settings.json");
    let p = UiPreferences {
        usage_bar_style: UsageBarStyle::FineTicks,
        ascii: true,
        theme: ThemeMode::Terminal,
        color: ColorPolicy::Never,
        reduced_motion: true,
        ..UiPreferences::default()
    };
    atomic_save(&path, &p).unwrap();
    let before = fs::read(&path).unwrap();
    for cwd in [&dir, &config] {
        let out = Command::new(env!("CARGO_BIN_EXE_polaris"))
            .env("POLARIS_CONFIG_DIR", &config)
            .current_dir(cwd)
            .args(["--scenario", "usage-demo", "--dump", "120x40"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{:?}", out.stderr);
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.is_ascii());
        assert!(text.contains('+') && text.contains("DEMO DATA"));
    }
    let out = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .env("POLARIS_CONFIG_DIR", &config)
        .args([
            "--theme",
            "dark",
            "--color",
            "always",
            "--scenario",
            "settings",
            "--dump",
            "120x40",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("launch override"));
    assert!(text.contains("[  [x] Dark ]"));
    assert_eq!(fs::read(&path).unwrap(), before);
}
