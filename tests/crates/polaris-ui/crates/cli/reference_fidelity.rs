//! Acceptance checks derived from dist's actual 80x24 buffer and design specification.
use polaris_sdk::{effort::Effort, ids::ModelId};
use polaris_ui_cli::screen::{
    CellDump, CompletionOverlay, DumpCell, InfoSheet, RenderOptions, ScreenState,
    composer_cursor_at, dump_cells, dump_text,
};
use polaris_ui_model::{
    completion::{CompletionKind, Suggestion},
    composer::Composer,
    view::ViewState,
};
use ratatui::style::Color;
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;
fn fixture() -> Result<(ViewState, Composer), Box<dyn Error>> {
    Ok((
        ViewState::new(ModelId::new("offline")?, Effort::Medium),
        Composer::new(),
    ))
}
fn cell(dump: &CellDump, x: u16, y: u16) -> Result<&DumpCell, Box<dyn Error>> {
    dump.cells
        .get(
            usize::from(y)
                .saturating_mul(usize::from(dump.width))
                .saturating_add(usize::from(x)),
        )
        .ok_or_else(|| "cell outside capture".into())
}
#[test]
fn welcome_matches_reference_composition() -> TestResult {
    let (view, composer) = fixture()?;
    let state = ScreenState::new(&view, &composer).with_workspace(Some("C:/workspace/polaris"));
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    let rows: Vec<_> = text.lines().collect();
    assert!(rows.get(2).is_some_and(|s| s.starts_with("    ✦ Polaris v") && s.contains("C:/workspace/polaris")), "{text}");
    assert_eq!(
        rows.get(14).map(|s| s.trim()),
        Some("What's on your mind for Polaris?")
    );
    assert!(
        rows.get(23).is_some_and(|s| s.contains("? for shortcuts")
            && s.contains("[ auto mode on ]")
            && s.contains("[ offline ]")
            && s.contains("[ medium ]")),
        "{text}"
    );
    assert!(!rows.get(3).is_some_and(|s| s.contains("medium")));
    Ok(())
}
#[test]
fn composer_has_open_black_interior_and_complete_rules() -> TestResult {
    let (view, composer) = fixture()?;
    let dump = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let area = dump.metadata.composer_rect;
    assert_eq!((area.x, area.y, area.width, area.height), (2, 19, 76, 3));
    for x in area.x..area.right() {
        assert_eq!(cell(&dump, x, area.y)?.symbol, "─");
        assert_eq!(cell(&dump, x, area.bottom().saturating_sub(1))?.symbol, "─");
        assert_eq!(
            cell(&dump, x, area.y.saturating_add(1))?.bg_rgb,
            Some((0, 0, 0))
        );
    }
    assert_eq!(cell(&dump, 4, 20)?.symbol, ">");
    assert_eq!(cell(&dump, 6, 20)?.fg_rgb, Some((82, 82, 82)));
    let caret = dump.metadata.caret.ok_or("missing caret")?;
    assert_eq!((caret.x, caret.y), (6, 20));
    assert_eq!(
        composer_cursor_at(&composer, area, 6, 19),
        None,
        "rule is not editable"
    );
    Ok(())
}
#[test]
fn composer_grows_from_wrapped_rows_up_to_three_eighths() -> TestResult {
    let (view, mut composer) = fixture()?;
    for _ in 0..100 {
        composer.insert("界é words")?;
        composer.newline()?;
    }
    let original = composer.text().to_owned();
    let cursor = composer.cursor();
    for (w, h) in [
        (80, 24),
        (160, 60),
        (60, 20),
        (100, 30),
        (120, 100),
        (80, 24),
    ] {
        let dump = dump_cells(
            &ScreenState::new(&view, &composer),
            &RenderOptions::default(),
            w,
            h,
        )?;
        assert_eq!(
            dump.metadata.composer_rect.height.saturating_sub(2),
            h.saturating_mul(3)
                .checked_div(8)
                .ok_or("invalid divisor")?
        );
        let caret = dump.metadata.caret.ok_or("missing caret")?;
        assert!(
            caret.y > dump.metadata.composer_rect.y
                && caret.y < dump.metadata.composer_rect.bottom().saturating_sub(1)
        );
        assert_eq!(composer.text(), original);
        assert_eq!(composer.cursor(), cursor);
    }
    let mut wrapped = Composer::new();
    wrapped.insert(&"x".repeat(240))?;
    let dump = dump_cells(
        &ScreenState::new(&view, &wrapped),
        &RenderOptions::default(),
        80,
        24,
    )?;
    assert_eq!(
        dump.metadata.composer_rect.height, 6,
        "four wrapped rows plus two rules"
    );
    Ok(())
}
#[test]
fn welcome_strokes_keep_reference_center_ray_and_tip_colors() -> TestResult {
    let (view, composer) = fixture()?;
    let dump = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    assert_eq!(cell(&dump, 39, 10)?.symbol, "✦");
    assert_eq!(cell(&dump, 39, 10)?.fg_rgb, Some((192, 229, 140)));
    assert_eq!(cell(&dump, 38, 10)?.fg_rgb, Some((118, 185, 0)));
    assert_eq!(cell(&dump, 35, 10)?.fg_rgb, Some((107, 107, 107)));
    Ok(())
}
#[test]
fn completion_keeps_selected_candidate_visible_on_black_surface() -> TestResult {
    let (view, mut composer) = fixture()?;
    composer.insert("/")?;
    let items: Vec<_> = (0..24)
        .map(|i| Suggestion {
            label: format!("/candidate{i:02}"),
            insert: format!("/candidate{i:02}"),
            description: "command description",
            range: 0..1,
            kind: CompletionKind::Command,
        })
        .collect();
    let state = ScreenState::new(&view, &composer).with_completion(Some(CompletionOverlay {
        title: "Commands",
        items: &items,
        selected: Some(19),
    }));
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    assert!(text.contains("/candidate19"), "{text}");
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    for c in &dump.cells {
        assert_eq!(c.bg, Color::Rgb(0, 0, 0));
    }
    assert!(dump.metadata.caret.is_none());
    Ok(())
}
#[test]
fn information_sheet_keeps_black_surface_and_pinned_hints() -> TestResult {
    let (view, composer) = fixture()?;
    let lines: Vec<_> = (0..100).map(|i| format!("Line {i}")).collect();
    let state = ScreenState::new(&view, &composer).with_sheet(Some(InfoSheet {
        title: "Status",
        lines: &lines,
        footer: Some("Enter/Esc close"),
        scroll: 20,
    }));
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    assert!(text.contains("Line 20") && text.contains("Enter/Esc close"));
    for c in &dump.cells {
        assert_eq!(c.bg, Color::Rgb(0, 0, 0));
    }
    // The pinned hint must occupy an interior row, never replace the bottom rule.
    let rows: Vec<_> = text.lines().collect();
    let hint = rows
        .iter()
        .position(|s| s.contains("Enter/Esc close"))
        .ok_or("hint missing")?;
    assert!(
        rows.get(hint.saturating_add(1))
            .is_some_and(|s| s.contains('─'))
    );
    Ok(())
}

#[test]
fn ansi_and_terminal_preserve_reference_semantic_colors() -> TestResult {
    use polaris_ui_cli::screen::ColorMode;
    let (view, composer) = fixture()?;
    for (mode, mode_color, placeholder) in [
        (ColorMode::Ansi, Color::Indexed(13), Color::Indexed(8)),
        (ColorMode::Terminal, Color::LightMagenta, Color::DarkGray),
    ] {
        let options = RenderOptions {
            colors: mode,
            ascii: false,
        };
        let dump = dump_cells(&ScreenState::new(&view, &composer), &options, 80, 24)?;
        assert_eq!(cell(&dump, 6, 20)?.fg, placeholder);
        assert!(dump.cells.iter().any(|c| c.fg == mode_color));
        let items = vec![Suggestion {
            label: "/help".to_owned(),
            insert: "/help".to_owned(),
            description: "Help",
            range: 0..1,
            kind: CompletionKind::Command,
        }];
        let screen = ScreenState::new(&view, &composer).with_completion(Some(CompletionOverlay {
            title: "Commands",
            items: &items,
            selected: Some(0),
        }));
        let dump = dump_cells(&screen, &options, 80, 24)?;
        let focused = if mode == ColorMode::Ansi {
            Color::Indexed(10)
        } else {
            Color::LightGreen
        };
        assert!(
            dump.cells
                .iter()
                .any(|c| c.symbol == "/" && c.fg == focused)
        );
    }
    Ok(())
}

#[test]
fn conversation_omits_role_headings_but_retains_error_labels() -> TestResult {
    let (mut view, composer) = fixture()?;
    view.push_user("A submitted request")?;
    view.push_assistant("An observed answer")?;
    view.push_error("An observed failure")?;
    let state = ScreenState::new(&view, &composer);
    let text = dump_text(&state, &RenderOptions::default(), 80, 24)?;
    assert!(
        !text
            .lines()
            .any(|row| matches!(row.trim(), "You" | "Polaris"))
    );
    assert!(text.lines().any(|row| row.trim() == "Error"));
    let dump = dump_cells(&state, &RenderOptions::default(), 80, 24)?;
    let span = dump
        .metadata
        .source_spans
        .iter()
        .find(|span| span.entry_index == 0 && !span.source_bytes.is_empty())
        .ok_or("user source absent")?;
    assert_eq!(
        cell(&dump, span.rect.x, span.rect.y)?.fg_rgb,
        Some((232, 232, 232))
    );
    assert_eq!(
        cell(&dump, span.rect.x, span.rect.y)?.bg_rgb,
        Some((20, 20, 20))
    );
    Ok(())
}

#[test]
fn export_reference_matrix() -> TestResult {
    let Some(dir) = std::env::var_os("POLARIS_UI_CAPTURE_DIR") else {
        return Ok(());
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let (view, composer) = fixture()?;
    for (w, h) in [(60, 20), (80, 24), (100, 30), (120, 40), (160, 60)] {
        let state = ScreenState::new(&view, &composer).with_workspace(Some("C:/workspace/polaris"));
        let dump = dump_cells(&state, &RenderOptions::default(), w, h)?;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\"><rect width=\"100%\" height=\"100%\" fill=\"#000\"/>",
            u32::from(w).saturating_mul(9),
            u32::from(h).saturating_mul(18)
        );
        for y in 0..h {
            for x in 0..w {
                let c = cell(&dump, x, y)?;
                if let Some((r, g, b)) = c.bg_rgb
                    && (r, g, b) != (0, 0, 0)
                {
                    svg.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"9\" height=\"18\" fill=\"#{r:02x}{g:02x}{b:02x}\"/>",u32::from(x).saturating_mul(9),u32::from(y).saturating_mul(18)));
                }
                if c.symbol.trim().is_empty() {
                    continue;
                }
                let (r, g, b) = c.fg_rgb.unwrap_or((232, 232, 232));
                let text = c
                    .symbol
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                svg.push_str(&format!("<text x=\"{}\" y=\"{}\" textLength=\"9\" lengthAdjust=\"spacingAndGlyphs\" font-family=\"Consolas,monospace\" font-size=\"15\" fill=\"#{r:02x}{g:02x}{b:02x}\">{text}</text>",u32::from(x).saturating_mul(9),u32::from(y).saturating_mul(18).saturating_add(14)));
            }
        }
        svg.push_str("</svg>");
        std::fs::write(dir.join(format!("welcome-{w}x{h}.svg")), svg)?;
        std::fs::write(
            dir.join(format!("welcome-{w}x{h}.txt")),
            dump_text(&state, &RenderOptions::default(), w, h)?,
        )?;
    }
    Ok(())
}
