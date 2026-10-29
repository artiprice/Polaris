//! Model replies as display blocks: headings, paragraphs, list items, code and diffs, with
//! inline `code` spans. Raw HTML is never produced; every piece is text for the view.

/// One inline run of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text (`**` markers removed).
    Text(String),
    /// A `code` span.
    Code(String),
}

/// One display block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// `#`–`######` heading.
    Heading(Vec<Inline>),
    /// Paragraph lines.
    Paragraph(Vec<Vec<Inline>>),
    /// `-`, `*`, `+` or `1.` item.
    Item(Vec<Inline>),
    /// Fenced code with its language label.
    Code {
        /// Fence language, or empty.
        language: String,
        /// Lines, verbatim.
        lines: Vec<String>,
    },
}

/// Diff line class for a code line in a `diff`/`patch` fence: `add`, `del` or empty.
#[must_use]
pub fn diff_class(language: &str, line: &str) -> &'static str {
    if !matches!(language, "diff" | "patch") || line.starts_with("+++") || line.starts_with("---") {
        ""
    } else if line.starts_with('+') {
        "add"
    } else if line.starts_with('-') {
        "del"
    } else {
        ""
    }
}

/// Split a line into text and `code` runs.
#[must_use]
pub fn inline(text: &str) -> Vec<Inline> {
    text.split('`')
        .enumerate()
        .filter(|(_, part)| !part.is_empty())
        .map(|(index, part)| {
            if index % 2 == 1 {
                Inline::Code(part.to_owned())
            } else {
                Inline::Text(part.replace("**", ""))
            }
        })
        .collect()
}

fn list_item(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0)
        .then(|| trimmed.get(digits..))
        .flatten()
        .and_then(|rest| rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")))
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) {
        line.get(hashes..)?.strip_prefix(' ')
    } else {
        None
    }
}

/// Split `text` into blocks; an unterminated fence (while streaming) is still code.
#[must_use]
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut paragraph: Vec<Vec<Inline>> = Vec::new();
    let mut code: Option<(String, Vec<String>)> = None;
    let flush = |paragraph: &mut Vec<Vec<Inline>>, out: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            out.push(Block::Paragraph(std::mem::take(paragraph)));
        }
    };
    for line in text.lines() {
        if let Some(fence) = line.trim_start().strip_prefix("```") {
            match code.take() {
                Some((language, lines)) => out.push(Block::Code { language, lines }),
                None => {
                    flush(&mut paragraph, &mut out);
                    code = Some((fence.trim().to_owned(), Vec::new()));
                }
            }
            continue;
        }
        if let Some((_, lines)) = code.as_mut() {
            lines.push(line.to_owned());
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut paragraph, &mut out);
        } else if let Some(title) = heading(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Heading(inline(title)));
        } else if let Some(item) = list_item(line) {
            flush(&mut paragraph, &mut out);
            out.push(Block::Item(inline(item)));
        } else {
            paragraph.push(inline(line));
        }
    }
    if let Some((language, lines)) = code {
        out.push(Block::Code { language, lines });
    }
    flush(&mut paragraph, &mut out);
    out
}
