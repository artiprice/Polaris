//! Lightweight lexical code colors and source-preserving unified-diff gutters.
use crate::theme::Theme;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
#[derive(Default)]
pub struct CodeBlock {
    language: String,
    line: usize,
    old: usize,
    new: usize,
}
pub struct CodeRow {
    pub line: Line<'static>,
    pub prefix: String,
}
impl CodeBlock {
    pub fn new(language: &str) -> Self {
        Self {
            language: language.trim().to_ascii_lowercase(),
            line: 1,
            old: 1,
            new: 1,
        }
    }
    pub fn row(&mut self, raw: &str, theme: Theme) -> CodeRow {
        let diff = matches!(self.language.as_str(), "diff" | "patch");
        let metadata = diff
            && (raw.starts_with("@@")
                || raw.starts_with("--- ")
                || raw.starts_with("+++ ")
                || raw.starts_with("diff ")
                || raw.starts_with("index ")
                || raw.starts_with('\\'));
        if diff && raw.starts_with("@@") {
            let parts = raw.split_whitespace().collect::<Vec<_>>();
            self.old = parts
                .iter()
                .find_map(|s| {
                    s.strip_prefix('-')
                        .and_then(|s| s.split(',').next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(self.old);
            self.new = parts
                .iter()
                .find_map(|s| {
                    s.strip_prefix('+')
                        .and_then(|s| s.split(',').next())
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(self.new);
        }
        let added = diff && !metadata && raw.starts_with('+');
        let removed = diff && !metadata && raw.starts_with('-');
        let number = if diff {
            if removed { self.old } else { self.new }
        } else {
            self.line
        };
        let prefix = if metadata {
            "       ".into()
        } else {
            format!("  {number:>4} ")
        };
        let surface = if added {
            theme.diff_added()
        } else if removed {
            theme.diff_removed()
        } else {
            theme.base()
        };
        let mut spans = vec![Span::styled(
            prefix.clone(),
            theme.muted().patch(surface).fg(if added {
                theme.diff_added().fg.unwrap_or(Color::Reset)
            } else if removed {
                theme.diff_removed().fg.unwrap_or(Color::Reset)
            } else {
                theme.muted().fg.unwrap_or(Color::Reset)
            }),
        )];
        let expanded = expand_tabs(raw, prefix.len());
        if metadata {
            spans.push(Span::styled(expanded, theme.muted()));
        } else {
            let marker = usize::from(diff && raw.starts_with(['+', '-', ' ']));
            if marker > 0 {
                spans.push(Span::styled(expanded[..1].to_owned(), surface));
            }
            spans.extend(highlight(
                &expanded[marker..],
                if diff { "rust" } else { &self.language },
                theme,
            ));
            if diff {
                if !added {
                    self.old += 1;
                }
                if !removed {
                    self.new += 1;
                }
            } else {
                self.line += 1;
            }
        }
        CodeRow {
            prefix,
            line: Line::from(spans).style(surface),
        }
    }
}
fn expand_tabs(raw: &str, mut column: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut out = String::new();
    for g in raw.graphemes(true) {
        if g == "\t" {
            let n = 4 - column % 4;
            out.push_str(&" ".repeat(n));
            column += n;
        } else {
            out.push_str(g);
            column += UnicodeWidthStr::width(g);
        }
    }
    out
}
fn tint(theme: Theme, rgb: (u8, u8, u8), ansi: Color) -> Style {
    if !theme.color {
        Style::default()
    } else {
        Style::default().fg(
            if theme.rgb && theme.mode == crate::theme::ThemeMode::Dark {
                Color::Rgb(rgb.0, rgb.1, rgb.2)
            } else {
                ansi
            },
        )
    }
}
fn highlight(raw: &str, language: &str, theme: Theme) -> Vec<Span<'static>> {
    if !matches!(
        language,
        "rust"
            | "rs"
            | "python"
            | "py"
            | "javascript"
            | "js"
            | "typescript"
            | "ts"
            | "json"
            | "toml"
            | "bash"
            | "sh"
            | "shell"
    ) {
        return vec![Span::raw(raw.to_owned())];
    }
    let mut out = vec![];
    let mut i = 0;
    while i < raw.len() {
        let tail = &raw[i..];
        let ch = tail.chars().next().unwrap();
        if tail.starts_with("//")
            || (ch == '#' && matches!(language, "python" | "py" | "bash" | "sh" | "shell" | "toml"))
        {
            out.push(Span::styled(
                tail.to_owned(),
                Style::default().fg(theme.muted().fg.unwrap_or(Color::Reset)),
            ));
            break;
        }
        let mut end = i + ch.len_utf8();
        let style = if ch == '"' || (ch == '\'' && tail[1..].contains('\'')) {
            let mut escaped = false;
            for (offset, c) in tail[ch.len_utf8()..].char_indices() {
                end = i + ch.len_utf8() + offset + c.len_utf8();
                if c == ch && !escaped {
                    break;
                }
                escaped = c == '\\' && !escaped;
            }
            tint(theme, (182, 227, 91), Color::LightGreen)
        } else if ch.is_ascii_digit() {
            while end < raw.len() {
                let c = raw[end..].chars().next().unwrap();
                if !(c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                    break;
                }
                end += c.len_utf8();
            }
            tint(theme, (249, 226, 175), Color::Yellow)
        } else if ch.is_alphabetic() || ch == '_' {
            while end < raw.len() {
                let c = raw[end..].chars().next().unwrap();
                if !(c.is_alphanumeric() || c == '_') {
                    break;
                }
                end += c.len_utf8();
            }
            let word = &raw[i..end];
            if matches!(
                word,
                "pub"
                    | "fn"
                    | "let"
                    | "mut"
                    | "const"
                    | "static"
                    | "use"
                    | "mod"
                    | "impl"
                    | "trait"
                    | "struct"
                    | "enum"
                    | "match"
                    | "if"
                    | "else"
                    | "return"
                    | "for"
                    | "while"
                    | "loop"
                    | "in"
                    | "as"
                    | "async"
                    | "await"
                    | "move"
                    | "self"
                    | "Self"
                    | "true"
                    | "false"
                    | "None"
                    | "Some"
                    | "def"
                    | "class"
                    | "import"
                    | "from"
                    | "and"
                    | "or"
                    | "not"
                    | "function"
                    | "export"
                    | "default"
                    | "new"
                    | "null"
                    | "undefined"
                    | "throw"
                    | "try"
                    | "catch"
                    | "type"
                    | "interface"
            ) {
                tint(theme, (203, 166, 247), Color::LightMagenta)
            } else if ch.is_uppercase() {
                tint(theme, (137, 220, 235), Color::LightCyan)
            } else {
                Style::default()
            }
        } else {
            Style::default()
        };
        out.push(Span::styled(raw[i..end].to_owned(), style));
        i = end;
    }
    out
}
