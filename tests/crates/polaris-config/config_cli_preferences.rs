//! CLI UI preference descriptors: specs/descriptors/config_cli_preferences.json.
use polaris_config::{
    CliColorDepth, CliColorPolicy, CliPanelDefault, CliTheme, Dirs, Layer, LayerInput,
    PreferenceField, PreferenceValue, TrustGrant, load, read_global_cli_preferences,
    save_global_cli_preference,
};
use std::fs;

#[test]
fn legacy_ui_theme_remains_independent() -> Result<(), Box<dyn std::error::Error>> {
    let input = LayerInput::parse(
        Layer::Global,
        "[ui]\ntheme = \"light\"\n[ui.cli]\ntheme = \"terminal\"",
        None,
    )?;
    let loaded = load(&[input], &TrustGrant::none())?;
    assert_eq!(loaded.config.ui.theme, polaris_config::Theme::Light);
    assert_eq!(loaded.config.ui.cli.theme, CliTheme::Terminal);
    Ok(())
}

#[test]
fn cli_preferences_default_and_validate() -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load(&[], &TrustGrant::none())?;
    assert_eq!(loaded.config.ui.cli.theme, CliTheme::Dark);
    assert_eq!(loaded.config.ui.cli.color, CliColorPolicy::Auto);
    assert_eq!(loaded.config.ui.cli.color_depth, CliColorDepth::Auto);
    assert_eq!(loaded.config.ui.cli.panel, CliPanelDefault::Closed);
    let invalid = LayerInput::parse(Layer::Global, "[ui.cli]\ncolor = \"red\"", None)?;
    assert!(load(&[invalid], &TrustGrant::none()).is_err());
    Ok(())
}

#[test]
fn global_preference_update_preserves_unknown_keys() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let dirs = Dirs {
        global: directory.path().to_path_buf(),
        repo: None,
    };
    fs::write(
        directory.path().join("config.toml"),
        "[unrecognized.future]\nkeep = \"yes\"\n[ui]\ntheme = \"light\"\n[ui.cli]\nascii = false\nreduce_motion = true\n",
    )?;
    save_global_cli_preference(
        &dirs,
        PreferenceField::Theme,
        PreferenceValue::Theme(CliTheme::Terminal),
    )?;
    let text = fs::read_to_string(directory.path().join("config.toml"))?;
    assert!(text.contains("keep = \"yes\""));
    assert!(text.contains("theme = \"light\""));
    assert!(text.contains("theme = \"terminal\""));
    assert!(text.contains("reduce_motion = true"));
    let loaded = read_global_cli_preferences(&dirs)?;
    assert_eq!(loaded.theme, CliTheme::Terminal);
    assert!(loaded.reduce_motion);
    Ok(())
}

#[test]
fn global_preference_write_is_atomic_on_failure() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let dirs = Dirs {
        global: directory.path().to_path_buf(),
        repo: None,
    };
    let config = directory.path().join("config.toml");
    let original = b"[ui.cli\ninvalid";
    fs::write(&config, original)?;
    let result = save_global_cli_preference(
        &dirs,
        PreferenceField::Ascii,
        PreferenceValue::Boolean(true),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&config)?, original);
    Ok(())
}

#[test]
fn only_requested_field_is_updated() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let dirs = Dirs {
        global: directory.path().to_path_buf(),
        repo: None,
    };
    save_global_cli_preference(
        &dirs,
        PreferenceField::Color,
        PreferenceValue::Color(CliColorPolicy::Never),
    )?;
    let loaded = read_global_cli_preferences(&dirs)?;
    assert_eq!(loaded.color, CliColorPolicy::Never);
    assert_eq!(loaded.theme, CliTheme::Dark);
    assert_eq!(loaded.panel, CliPanelDefault::Closed);
    Ok(())
}
