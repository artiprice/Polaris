//! Documentation ingestion without the network: `llms.txt` link lists, HTML pages converted to
//! Markdown, Markdown split into heading-scoped chunks, and a BM25 index queried per topic.
//! The offline seed is a JSON-lines file of chunks, built by xtask as a release artifact.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use crate::NvidiaError;
use crate::profile::DocTopic;

/// Largest document accepted, in bytes.
pub const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
/// Most chunks one index holds.
pub const MAX_CHUNKS: usize = 200_000;

/// One link of an `llms.txt` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocLink {
    /// Section heading the link sits under (empty before the first `##`).
    pub section: String,
    /// Link text.
    pub title: String,
    /// Target URL.
    pub url: String,
    /// Text after the link, if any.
    pub description: String,
}

/// A parsed `llms.txt`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmsTxt {
    /// The `# ` title.
    pub title: String,
    /// The `> ` summary.
    pub summary: String,
    /// Links in order.
    pub links: Vec<DocLink>,
}

/// Parse the `llms.txt` convention: `# Title`, `> summary`, `## Section`, `- [title](url): text`.
#[must_use]
pub fn parse_llms_txt(text: &str) -> LlmsTxt {
    let mut parsed = LlmsTxt::default();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("# ") {
            if parsed.title.is_empty() {
                title.trim().clone_into(&mut parsed.title);
            }
        } else if let Some(summary) = line.strip_prefix("> ") {
            if !parsed.summary.is_empty() {
                parsed.summary.push(' ');
            }
            parsed.summary.push_str(summary.trim());
        } else if let Some(heading) = line.strip_prefix("## ") {
            heading.trim().clone_into(&mut section);
        } else if let Some(item) = line
            .strip_prefix("- [")
            .or_else(|| line.strip_prefix("* ["))
        {
            let Some((title, rest)) = item.split_once("](") else {
                continue;
            };
            let Some((url, tail)) = rest.split_once(')') else {
                continue;
            };
            parsed.links.push(DocLink {
                section: section.clone(),
                title: title.trim().to_owned(),
                url: url.trim().to_owned(),
                description: tail.trim_start_matches(':').trim().to_owned(),
            });
        }
    }
    parsed
}

/// Convert an HTML page to Markdown.
/// # Errors
/// Oversized input or a conversion failure.
pub fn html_to_markdown(html: &str) -> Result<String, NvidiaError> {
    if html.len() > MAX_DOCUMENT_BYTES {
        return Err(NvidiaError::TooLarge("HTML page"));
    }
    htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "nav", "footer", "header"])
        .build()
        .convert(html)
        .map_err(|_| NvidiaError::Malformed("HTML page"))
}

/// A heading-scoped piece of one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocChunk {
    /// Documentation set.
    pub topic: String,
    /// Source URL or path.
    pub source: String,
    /// Heading path, outermost first, joined with ` › `.
    pub heading: String,
    /// Markdown text of the chunk.
    pub text: String,
}

/// Split Markdown into chunks of at most `max_chars` characters, each under its heading path.
/// Code blocks are never split.
/// # Errors
/// Oversized input.
pub fn chunk_markdown(
    markdown: &str,
    topic: DocTopic,
    source: &str,
    max_chars: usize,
) -> Result<Vec<DocChunk>, NvidiaError> {
    if markdown.len() > MAX_DOCUMENT_BYTES {
        return Err(NvidiaError::TooLarge("Markdown document"));
    }
    // Heading positions from a real CommonMark parse, so `#` inside code is not a heading.
    let mut headings: Vec<(usize, usize, usize, String)> = Vec::new();
    let mut current: Option<(usize, usize, String)> = None;
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some((heading_depth(level), range.start, String::new()));
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, title)) = current.as_mut() {
                    title.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((depth, start, title)) = current.take() {
                    headings.push((depth, start, range.end, title.trim().to_owned()));
                }
            }
            _ => {}
        }
    }
    let mut chunks = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut section_start = 0_usize;
    let emit = |path: &[(usize, String)], text: &str, chunks: &mut Vec<DocChunk>| {
        let heading = path
            .iter()
            .map(|(_, title)| title.as_str())
            .collect::<Vec<_>>()
            .join(" › ");
        for piece in split_section(text, max_chars.max(200)) {
            chunks.push(DocChunk {
                topic: topic.as_str().to_owned(),
                source: source.to_owned(),
                heading: heading.clone(),
                text: piece,
            });
        }
    };
    for (depth, start, end, title) in headings {
        emit(
            &path,
            markdown.get(section_start..start).unwrap_or_default(),
            &mut chunks,
        );
        while path.last().is_some_and(|(level, _)| *level >= depth) {
            path.pop();
        }
        path.push((depth, title));
        section_start = end;
    }
    emit(
        &path,
        markdown.get(section_start..).unwrap_or_default(),
        &mut chunks,
    );
    Ok(chunks)
}

const fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Blank-line-separated blocks packed into pieces of at most `max_chars`; a fenced code block is
/// one block however long.
fn split_section(text: &str, max_chars: usize) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut block = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if line.trim().is_empty() && !in_fence {
            if !block.trim().is_empty() {
                blocks.push(std::mem::take(&mut block));
            }
            block.clear();
            continue;
        }
        block.push_str(line);
        block.push('\n');
    }
    if !block.trim().is_empty() {
        blocks.push(block);
    }
    let mut pieces = Vec::new();
    let mut piece = String::new();
    for block in blocks {
        if !piece.is_empty()
            && piece.chars().count().saturating_add(block.chars().count()) > max_chars
        {
            pieces.push(std::mem::take(&mut piece).trim_end().to_owned());
        }
        if !piece.is_empty() {
            piece.push('\n');
        }
        piece.push_str(&block);
    }
    if !piece.trim().is_empty() {
        pieces.push(piece.trim_end().to_owned());
    }
    pieces
}

/// A ranked chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct DocHit {
    /// BM25 score.
    pub score: f64,
    /// The chunk.
    pub chunk: DocChunk,
}

/// In-memory BM25 index over documentation chunks.
#[derive(Debug, Default)]
pub struct DocIndex {
    chunks: Vec<DocChunk>,
    lengths: Vec<usize>,
    postings: BTreeMap<String, Vec<(usize, u32)>>,
    total_length: usize,
}

const K1: f64 = 1.2;
const B: f64 = 0.75;
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "are", "you", "how", "what", "can", "use", "from",
    "into", "not", "its", "per",
];

fn terms(text: &str) -> Vec<String> {
    text.split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .filter(|word| word.len() > 1)
        .map(str::to_lowercase)
        .filter(|word| !STOP_WORDS.contains(&word.as_str()))
        .collect()
}

impl DocIndex {
    /// Number of chunks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Add one chunk (its heading counts as text).
    /// # Errors
    /// The index is full.
    pub fn add(&mut self, chunk: DocChunk) -> Result<(), NvidiaError> {
        if self.chunks.len() >= MAX_CHUNKS {
            return Err(NvidiaError::TooLarge("documentation index"));
        }
        let id = self.chunks.len();
        let words = terms(&format!("{} {}", chunk.heading, chunk.text));
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for word in &words {
            let count = counts.entry(word.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
        for (word, count) in counts {
            self.postings.entry(word).or_default().push((id, count));
        }
        self.lengths.push(words.len());
        self.total_length = self.total_length.saturating_add(words.len());
        self.chunks.push(chunk);
        Ok(())
    }

    /// The best `limit` chunks for `query`, optionally within one topic.
    #[must_use]
    pub fn query(&self, topic: Option<DocTopic>, query: &str, limit: usize) -> Vec<DocHit> {
        if self.chunks.is_empty() {
            return Vec::new();
        }
        let count = self.chunks.len() as f64;
        let average = (self.total_length as f64 / count).max(1.0);
        let mut scores: BTreeMap<usize, f64> = BTreeMap::new();
        let unique: BTreeSet<String> = terms(query).into_iter().collect();
        for term in unique {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let frequency = postings.len() as f64;
            let idf = ((count - frequency + 0.5) / (frequency + 0.5) + 1.0).ln();
            for (id, occurrences) in postings {
                if topic.is_some_and(|topic| {
                    self.chunks
                        .get(*id)
                        .is_none_or(|chunk| chunk.topic != topic.as_str())
                }) {
                    continue;
                }
                let length = self.lengths.get(*id).copied().unwrap_or_default() as f64;
                let tf = f64::from(*occurrences);
                let score = idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * length / average));
                *scores.entry(*id).or_insert(0.0) += score;
            }
        }
        let mut ranked: Vec<(usize, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                self.chunks.get(id).map(|chunk| DocHit {
                    score,
                    chunk: chunk.clone(),
                })
            })
            .collect()
    }

    /// Write every chunk as one JSON line (the offline seed format).
    /// # Errors
    /// The writer failed.
    pub fn write_seed(&self, mut writer: impl Write) -> Result<(), NvidiaError> {
        for chunk in &self.chunks {
            serde_json::to_writer(&mut writer, chunk).map_err(|_| NvidiaError::Seed)?;
            writer.write_all(b"\n").map_err(|_| NvidiaError::Seed)?;
        }
        Ok(())
    }

    /// Load chunks from a JSON-lines seed.
    /// # Errors
    /// An unreadable line, a malformed chunk or an unknown topic.
    pub fn read_seed(reader: impl BufRead) -> Result<Self, NvidiaError> {
        let mut index = Self::default();
        for line in reader.lines() {
            let line = line.map_err(|_| NvidiaError::Seed)?;
            if line.trim().is_empty() {
                continue;
            }
            let chunk: DocChunk = serde_json::from_str(&line).map_err(|_| NvidiaError::Seed)?;
            if DocTopic::parse(&chunk.topic).is_none() {
                return Err(NvidiaError::Seed);
            }
            index.add(chunk)?;
        }
        Ok(index)
    }
}
