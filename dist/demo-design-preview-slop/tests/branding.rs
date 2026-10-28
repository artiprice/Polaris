use polaris::{app::App, theme::Theme, ui};
use std::process::Command;
#[test]
fn executable_package_help_version_and_failures_use_polaris() {
    assert_eq!(env!("CARGO_PKG_NAME"), "polaris");
    let exe = env!("CARGO_BIN_EXE_polaris");
    let version = Command::new(exe).arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        "Polaris v0.1.0alpha"
    );
    let help = Command::new(exe).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.starts_with("polaris ·"));
    assert!(help.contains("Usage: polaris"));
    assert!(!help.contains("nvvi"));
    let bad = Command::new(exe).arg("--not-an-option").output().unwrap();
    assert!(!bad.status.success());
    let error = String::from_utf8(bad.stderr).unwrap();
    assert!(error.starts_with("polaris:"));
}
#[test]
fn product_title_placeholder_and_vendor_identity_are_consistent() {
    let app = App::default();
    let screen = ui::dump(&app, 80, 24).unwrap();
    assert!(screen.contains("Polaris v0.1.0alpha"));
    assert!(screen.contains("What's on your mind for Polaris?"));
    assert!(screen.contains("Nemotron Super"));
    let mut status = App::default();
    status.command("/status");
    assert!(ui::dump(&status, 80, 24).unwrap().contains("NVIDIA NIM"));
    assert!(!screen.contains("Nvvi Code"));
    let t = Theme::default();
    assert_eq!(t.error().fg, Some(ratatui::style::Color::Rgb(172, 66, 66)));
    assert_eq!(t.accent().fg, Some(ratatui::style::Color::Rgb(118, 185, 0)));
}
#[test]
fn legacy_palette_and_original_references_are_preserved() {
    let palette = include_str!("../docs/branding/legacy-nvvi/palette.json");
    for value in [
        "d0a419195b05d541798b05922ce9b847bdd0b71c",
        "#76B900",
        "#B6E35B",
        "#AC4242",
        "#1C1C1C",
        "#303030",
    ] {
        assert!(palette.contains(value));
    }
    assert!(include_str!("../docs/branding/legacy-nvvi/logo.rs.txt").contains("──╢ │ NV  │ ╟──"));
    assert!(include_str!("../docs/branding/legacy-nvvi/theme.rs.txt").contains("(172, 66, 66)"));
    for image in [
        include_bytes!("../assets/branding/references/star-primary.png").as_slice(),
        include_bytes!("../assets/branding/references/star-secondary.png").as_slice(),
    ] {
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(u32::from_be_bytes(image[16..20].try_into().unwrap()), 1024);
        assert_eq!(u32::from_be_bytes(image[20..24].try_into().unwrap()), 1024);
    }
}
