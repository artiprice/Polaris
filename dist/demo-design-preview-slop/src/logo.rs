//! Asset-free North Star and shared responsive header geometry.
use crate::{
    app::App,
    theme::{Theme, ThemeMode},
};
use ratatui::{
    Frame,
    buffer::Cell,
    layout::Rect,
    style::{Color, Modifier},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    Welcome,
    Compact,
    Emblem,
}
impl Variant {
    pub(crate) fn size(self) -> (u16, u16) {
        match self {
            Self::Welcome => (9, 5),
            Self::Compact => (5, 3),
            Self::Emblem => (1, 1),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct WelcomeLayout {
    pub art: Rect,
    pub heading: Rect,
    pub variant: Variant,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeaderLayout {
    pub bar: Rect,
    pub account: Option<Rect>,
    pub welcome: Option<WelcomeLayout>,
    pub history_y: u16,
}
pub(crate) fn layout(app: &App, area: Rect, body_height: u16) -> HeaderLayout {
    let bar = Rect::new(area.x + 2, area.y + 2, area.width.saturating_sub(4), 1);
    let account = app
        .account_email
        .as_ref()
        .map(|_| Rect::new(bar.x + 2, bar.y + 1, bar.width.saturating_sub(2), 1));
    let history_y = bar.y + 2 + u16::from(account.is_some());
    let available = area.y.saturating_add(body_height).saturating_sub(history_y);
    let welcome = if app.visible_entries().is_empty()
        && app.agents.records.is_empty()
        && !app.working
        && available >= 5
    {
        let variant = if available >= 9 {
            Variant::Welcome
        } else {
            Variant::Compact
        };
        let (w, h) = variant.size();
        let top = history_y + (available - h - 2) / 2;
        Some(WelcomeLayout {
            art: Rect::new(area.x + area.width.saturating_sub(w) / 2, top, w, h),
            heading: Rect::new(area.x, top + h + 1, area.width, 1),
            variant,
        })
    } else {
        None
    };
    HeaderLayout {
        bar,
        account,
        welcome,
        history_y,
    }
}
const WELCOME: [&str; 5] = [
    "    ╷    ",
    "    │    ",
    "╶───✦───╴",
    "    │    ",
    "    ╵    ",
];
const COMPACT: [&str; 3] = ["  ╷  ", "╶─✦─╴", "  ╵  "];
const EMBLEM: [&str; 1] = ["✦"];
fn rows(variant: Variant) -> &'static [&'static str] {
    match variant {
        Variant::Welcome => &WELCOME,
        Variant::Compact => &COMPACT,
        Variant::Emblem => &EMBLEM,
    }
}
fn stroke(glyph: char, x: usize, y: usize, variant: Variant, theme: Theme, ascii: bool) -> Cell {
    let (w, h) = variant.size();
    let (cx, cy) = (usize::from(w / 2), usize::from(h / 2));
    let near = x.abs_diff(cx) + y.abs_diff(cy) == 1;
    let foreground = if !theme.color {
        Color::Reset
    } else if theme.rgb && theme.mode == ThemeMode::Dark {
        if glyph == '✦' {
            Color::Rgb(192, 229, 140)
        } else if near {
            Color::Rgb(118, 185, 0)
        } else {
            Color::Rgb(107, 107, 107)
        }
    } else if glyph == '✦' {
        Color::LightGreen
    } else if near {
        Color::Green
    } else {
        Color::DarkGray
    };
    let symbol = if ascii {
        match glyph {
            '╷' | '│' | '╵' => '|',
            '╶' | '─' | '╴' => '-',
            '✦' => '*',
            other => other,
        }
    } else {
        glyph
    };
    let mut cell = Cell::default();
    cell.set_style(theme.base().remove_modifier(Modifier::BOLD))
        .set_char(symbol);
    if glyph != ' ' {
        cell.set_fg(foreground);
    }
    cell
}
pub(crate) fn render(frame: &mut Frame, rect: Rect, variant: Variant, theme: Theme, ascii: bool) {
    let visible = rect.intersection(frame.area());
    let artwork = rows(variant);
    for y in visible.y..visible.bottom() {
        for x in visible.x..visible.right() {
            let (xx, yy) = ((x - rect.x) as usize, (y - rect.y) as usize);
            if let Some(glyph) = artwork.get(yy).and_then(|r| r.chars().nth(xx)) {
                frame.buffer_mut()[(x, y)] = stroke(glyph, xx, yy, variant, theme, ascii);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;
    #[test]
    fn north_star_grids_have_exact_single_cell_geometry() {
        for v in [Variant::Welcome, Variant::Compact, Variant::Emblem] {
            let (w, h) = v.size();
            assert_eq!(rows(v).len(), h as usize);
            assert!(
                rows(v)
                    .iter()
                    .all(|r| UnicodeWidthStr::width(*r) == w as usize)
            );
            assert_eq!(
                rows(v)
                    .iter()
                    .map(|r| r.matches('✦').count())
                    .sum::<usize>(),
                1
            );
            assert_eq!(
                rows(v)[h as usize / 2].chars().nth(w as usize / 2),
                Some('✦')
            );
            assert!(!rows(v).join("").chars().any(|c| "┌┐└┘┬┴├┤█".contains(c)));
        }
        assert_eq!(
            WELCOME,
            [
                "    ╷    ",
                "    │    ",
                "╶───✦───╴",
                "    │    ",
                "    ╵    "
            ]
        );
        assert_eq!(COMPACT, ["  ╷  ", "╶─✦─╴", "  ╵  "]);
    }
    #[test]
    fn color_ascii_and_native_fallbacks_are_unfilled_regular_and_cell_stable() {
        for color in [true, false] {
            for rgb in [true, false] {
                for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
                    for ascii in [true, false] {
                        let t = Theme { color, rgb, mode };
                        for v in [Variant::Welcome, Variant::Compact, Variant::Emblem] {
                            for (y, r) in rows(v).iter().enumerate() {
                                for (x, g) in r.chars().enumerate() {
                                    let c = stroke(g, x, y, v, t, ascii);
                                    assert_eq!(UnicodeWidthStr::width(c.symbol()), 1);
                                    assert!(
                                        !c.modifier.intersects(Modifier::BOLD | Modifier::REVERSED)
                                    );
                                    assert_eq!(c.bg, t.base().bg.unwrap_or(Color::Reset));
                                    if ascii {
                                        assert!(c.symbol().is_ascii());
                                    }
                                    if !color {
                                        assert_eq!(c.fg, Color::Reset);
                                    }
                                    if mode == ThemeMode::Terminal || !rgb {
                                        assert!(!matches!(c.fg, Color::Rgb(..)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn soft_center_green_rays_and_gray_tips_use_the_exact_palette() {
        let t = Theme::default();
        assert_eq!(
            stroke('✦', 4, 2, Variant::Welcome, t, false).fg,
            Color::Rgb(192, 229, 140)
        );
        assert_eq!(
            stroke('─', 3, 2, Variant::Welcome, t, false).fg,
            Color::Rgb(118, 185, 0)
        );
        assert_eq!(
            stroke('╶', 0, 2, Variant::Welcome, t, false).fg,
            Color::Rgb(107, 107, 107)
        );
    }
    #[test]
    fn welcome_depends_on_content_and_space_never_on_popup_state() {
        let mut app = App::default();
        let r = Rect::new(10, 0, 100, 40);
        let a = layout(&app, r, 35);
        assert_eq!(a.welcome.unwrap().variant, Variant::Welcome);
        app.command("/model");
        assert_eq!(
            layout(&app, r, 35).welcome.unwrap().art,
            a.welcome.unwrap().art
        );
        assert_eq!(
            layout(&app, r, 10).welcome.unwrap().variant,
            Variant::Compact
        );
        assert!(layout(&app, r, 8).welcome.is_none());
        app.entries.push(crate::app::Entry::new(
            crate::app::EntryKind::User,
            "",
            &["hi"],
        ));
        assert!(layout(&app, r, 35).welcome.is_none());
    }
}
