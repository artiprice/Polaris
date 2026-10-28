//! Foreground token accents and conservative web-link detection for terminal text.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Mention,
    Command,
    Setting,
    Link,
}
#[derive(Clone, Debug)]
pub struct TokenDecoration {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

fn web_link_len(text: &str) -> Option<usize> {
    let mut end = text
        .find(|ch: char| ch.is_whitespace() || ch.is_control() || "<>\"'".contains(ch))
        .unwrap_or(text.len());
    // Count once so long runs of closing punctuation remain linear to scan.
    let mut balance = [0isize; 3];
    for ch in text[..end].chars() {
        match ch {
            '(' => balance[0] -= 1,
            ')' => balance[0] += 1,
            '[' => balance[1] -= 1,
            ']' => balance[1] += 1,
            '{' => balance[2] -= 1,
            '}' => balance[2] += 1,
            _ => {}
        }
    }
    loop {
        let last = text[..end].chars().next_back()?;
        let closer = match last {
            ')' => Some(0),
            ']' => Some(1),
            '}' => Some(2),
            _ => None,
        };
        let unbalanced = closer.is_some_and(|index| balance[index] > 0);
        if ".,;:!?".contains(last) || unbalanced {
            end -= last.len_utf8();
            if let Some(index) = closer {
                balance[index] -= 1;
            }
        } else {
            break;
        }
    }
    let candidate = &text[..end];
    let lower = candidate.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        return (!rest.is_empty() && !rest.starts_with('/')).then_some(end);
    }
    let authority = candidate.split(['/', '?', '#']).next()?;
    // Email and filesystem references have their own semantics.
    if authority.contains(['@', '\\']) {
        return None;
    }
    let host = authority.split(':').next()?;
    let labels = host.split('.').collect::<Vec<_>>();
    if labels.len() < 2 {
        return None;
    }
    let tld = labels.last()?.to_ascii_lowercase();
    if !(2..=63).contains(&tld.len()) || !tld.bytes().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    // Avoid turning common filenames into URLs. Bare domains are a heuristic;
    // explicit http(s) and www addresses remain available for ambiguous suffixes.
    let source_suffix = [
        "rs", "py", "js", "ts", "jsx", "tsx", "json", "toml", "yaml", "yml", "md", "txt", "lock",
        "html", "css", "scss", "sh", "ps1", "exe", "dll", "png", "jpg", "jpeg", "gif", "svg",
        "pdf", "zip", "log", "csv", "xml",
    ]
    .contains(&tld.as_str());
    if source_suffix && !lower.starts_with("www.") {
        return None;
    }
    labels
        .iter()
        .all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
        })
        .then_some(end)
}

pub fn token_decorations(text: &str) -> Vec<TokenDecoration> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap();
        let previous = text[..i].chars().next_back();
        let boundary =
            i == 0 || previous.is_some_and(|p| p.is_whitespace() || "([{<\"'".contains(p));
        let mention = ch == '@' && (boundary || previous.is_some_and(|p| ",;".contains(p)));
        if mention {
            let mut end = i + 1;
            if text[end..].starts_with('"') {
                end += 1;
                while end < text.len() {
                    let next = text[end..].chars().next().unwrap();
                    if next == '\n' {
                        break;
                    }
                    end += next.len_utf8();
                    if next == '"' {
                        break;
                    }
                }
            } else {
                end += text[end..]
                    .find(char::is_whitespace)
                    .unwrap_or(text.len() - end);
            }
            out.push(TokenDecoration {
                range: i..end,
                kind: TokenKind::Mention,
            });
            i = end;
            continue;
        }
        if ch == '$' && boundary {
            let end = i
                + 1
                + text[i + 1..]
                    .find(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '-'))
                    .unwrap_or(text.len() - i - 1);
            if crate::prompt_settings::SETTINGS
                .iter()
                .any(|(name, _)| *name == &text[i..end])
            {
                out.push(TokenDecoration {
                    range: i..end,
                    kind: TokenKind::Setting,
                });
                i = end;
                continue;
            }
        }
        if ch == '/' && (i == 0 || previous.is_some_and(char::is_whitespace)) {
            let end = i + text[i..]
                .find(char::is_whitespace)
                .unwrap_or(text.len() - i);
            let token = text[i..end].trim_end_matches([',', '.', ':', '!', '?']);
            if crate::app::COMMANDS
                .iter()
                .any(|(command, _)| *command == token)
                || matches!(token, "/plan" | "/goal")
            {
                out.push(TokenDecoration {
                    range: i..i + token.len(),
                    kind: TokenKind::Command,
                });
                i = end;
                continue;
            }
        }
        if boundary
            && ch.is_ascii_alphanumeric()
            && let Some(len) = web_link_len(&text[i..])
        {
            out.push(TokenDecoration {
                range: i..i + len,
                kind: TokenKind::Link,
            });
            i += len;
            continue;
        }
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn links(text: &str) -> Vec<&str> {
        token_decorations(text)
            .iter()
            .filter(|t| t.kind == TokenKind::Link)
            .map(|t| &text[t.range.clone()])
            .collect()
    }
    #[test]
    fn recognizes_schemes_www_and_bare_domains_with_balanced_punctuation() {
        let text = "See https://example.com/a(b), (http://localhost:8080/x). www.example.org/docs and example.else?q=1!";
        assert_eq!(
            links(text),
            [
                "https://example.com/a(b)",
                "http://localhost:8080/x",
                "www.example.org/docs",
                "example.else?q=1"
            ]
        );
        let long = format!("https://example.com/x{}", ")".repeat(10000));
        assert_eq!(links(&long), ["https://example.com/x"]);
    }
    #[test]
    fn does_not_color_files_paths_email_versions_or_mentions_as_links() {
        let text = "Cargo.toml src/main.rs /tmp/example.com C:\\src\\site.com foo.rs me@example.com v0.1.0 @example.com @\"a file.txt\"";
        assert!(links(text).is_empty());
        let tokens = token_decorations(text);
        assert_eq!(
            tokens
                .iter()
                .filter(|t| t.kind == TokenKind::Mention)
                .count(),
            2
        );
        assert!(links("http:// https:// /plan.txt /unknown").is_empty());
    }
    #[test]
    fn unicode_offsets_and_command_boundaries_are_exact() {
        let text = "漢字 example.com /plan, /plan.txt /goal @\"é file.txt\"";
        let tokens = token_decorations(text);
        assert_eq!(
            tokens
                .iter()
                .map(|t| (&text[t.range.clone()], t.kind))
                .collect::<Vec<_>>(),
            [
                ("example.com", TokenKind::Link),
                ("/plan", TokenKind::Command),
                ("/goal", TokenKind::Command),
                ("@\"é file.txt\"", TokenKind::Mention)
            ]
        );
    }
}
