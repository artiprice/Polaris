use ratatui::{
    buffer::Buffer,
    style::{Color, Modifier},
};
use unicode_width::UnicodeWidthStr;
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn ink(c: Color, background: bool) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => if background { "#000000" } else { "#e8e8e8" }.into(),
    }
}
pub fn export(name: &str, b: &Buffer) {
    let Some(folder) = std::env::var_os("POLARIS_CAPTURE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#000\"/>",
        b.area.width * 9,
        b.area.height * 18,
        b.area.width * 9,
        b.area.height * 18
    );
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            let c = &b[(x, y)];
            if matches!(c.bg, Color::Reset | Color::Black | Color::Rgb(0, 0, 0)) {
                continue;
            }
            svg += &format!(
                "<rect x=\"{}\" y=\"{}\" width=\"9\" height=\"18\" fill=\"{}\"/>",
                x * 9,
                y * 18,
                ink(c.bg, true)
            );
        }
    }
    for y in 0..b.area.height {
        for x in 0..b.area.width {
            let c = &b[(x, y)];
            if !c.symbol().trim().is_empty() {
                svg += &format!(
                    "<text x=\"{}\" y=\"{}\" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" font-family=\"Consolas,monospace\" font-size=\"15\" font-weight=\"{}\" text-decoration=\"{}\" fill=\"{}\">{}</text>",
                    x * 9,
                    y * 18 + 14,
                    UnicodeWidthStr::width(c.symbol()).max(1) * 9,
                    if c.modifier.contains(Modifier::BOLD) {
                        700
                    } else {
                        400
                    },
                    if c.modifier.contains(Modifier::UNDERLINED) {
                        "underline"
                    } else {
                        "none"
                    },
                    ink(c.fg, false),
                    xml(c.symbol())
                );
            }
        }
    }
    svg += "</svg>";
    std::fs::write(
        dir.join(format!("{name}-{}x{}.svg", b.area.width, b.area.height)),
        svg,
    )
    .unwrap();
}
