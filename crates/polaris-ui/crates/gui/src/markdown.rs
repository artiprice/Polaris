//! Assistant replies as display blocks: paragraphs, headings, list items and code. Inline
//! `**strong**` and `` `code` `` markers are removed; code keeps its exact text.

/// One displayable block of a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// "p", "h", "li" or "code".
    pub kind: &'static str,
    /// Text without Markdown markers (code is verbatim).
    pub text: String,
}

fn plain(text: &str) -> String {
    text.replace("**", "").replace('`', "")
}

fn list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let rest = trimmed.get(digits..)?;
        return rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "));
    }
    None
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) {
        line.get(hashes..)?.strip_prefix(' ')
    } else {
        None
    }
}

/// Split `text` into display blocks.
#[must_use]
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |paragraph: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            out.push(Block {
                kind: "p",
                text: plain(&paragraph.join("\n")),
            });
            paragraph.clear();
        }
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(lines) => out.push(Block {
                    kind: "code",
                    text: lines.join("\n"),
                }),
                None => {
                    flush(&mut paragraph, &mut out);
                    code = Some(Vec::new());
                }
            }
            continue;
        }
        if let Some(lines) = code.as_mut() {
            lines.push(line);
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut paragraph, &mut out);
        } else if let Some(title) = heading(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block {
                kind: "h",
                text: plain(title),
            });
        } else if let Some(item) = list_item(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block {
                kind: "li",
                text: plain(item),
            });
        } else {
            paragraph.push(line);
        }
    }
    if let Some(lines) = code {
        // An unterminated fence while streaming still shows as code.
        out.push(Block {
            kind: "code",
            text: lines.join("\n"),
        });
    }
    flush(&mut paragraph, &mut out);
    out
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/gui/src/markdown.rs"]
mod tests;
