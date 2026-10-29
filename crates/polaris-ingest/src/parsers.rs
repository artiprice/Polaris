use std::{
    io::{Cursor, Read},
    rc::Rc,
};

use calamine::{Reader, Xlsx, open_workbook_from_rs};
use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;
use polaris_sdk::registry::{DocBlock, IngestError};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use quick_xml::{Reader as XmlReader, events::Event as XmlEvent};
use zip::ZipArchive;

use crate::{
    DocKind, IngestLimits, IngestedDocument, LocatedBlock, SourceLocation, Trust, archive,
};

pub(super) fn dispatch(
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
    limits: IngestLimits,
) -> Result<IngestedDocument, IngestError> {
    if bytes.is_empty() {
        return Err(IngestError::Malformed("empty input".to_owned()));
    }
    let extension = name.rsplit('.').next().map(str::to_ascii_lowercase);
    let kind = if bytes.starts_with(b"%PDF-") {
        DocKind::Pdf
    } else if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        archive::validate(bytes, limits)?;
        zip_kind(bytes)?
    } else if infer::get(bytes).is_some_and(|kind| kind.matcher_type() == infer::MatcherType::Image)
    {
        DocKind::Image
    } else {
        match mime.map(str::to_ascii_lowercase).as_deref() {
            Some("text/html") => DocKind::Html,
            Some("text/markdown") => DocKind::Markdown,
            Some("text/csv" | "text/tab-separated-values") => DocKind::Csv,
            Some("text/plain") => DocKind::Text,
            _ => match extension.as_deref() {
                Some("html" | "htm") => DocKind::Html,
                Some("md" | "markdown") => DocKind::Markdown,
                Some("csv" | "tsv") => DocKind::Csv,
                Some("txt" | "log" | "rst") => DocKind::Text,
                Some("pdf") => DocKind::Pdf,
                Some("docx") => {
                    archive::validate(bytes, limits)?;
                    DocKind::Docx
                }
                Some("xlsx") => {
                    archive::validate(bytes, limits)?;
                    DocKind::Xlsx
                }
                Some("pptx") => {
                    archive::validate(bytes, limits)?;
                    DocKind::Pptx
                }
                _ => {
                    return Err(IngestError::Unsupported(format!(
                        "unrecognized document kind for {name}"
                    )));
                }
            },
        }
    };
    if matches!(kind, DocKind::Docx | DocKind::Xlsx | DocKind::Pptx) {
        validate_ooxml_xml_parts(bytes, limits)?;
    }
    let (title, blocks) = match kind {
        DocKind::Text => (None, parse_text(bytes)?),
        DocKind::Markdown => {
            let text = decode_text(bytes)?;
            (None, parse_markdown(&text)?)
        }
        DocKind::Html => {
            let text = decode_text(bytes)?;
            parse_html(&text, limits)?
        }
        DocKind::Csv => {
            let delimiter = match mime.map(str::to_ascii_lowercase).as_deref() {
                Some("text/tab-separated-values") => b'\t',
                _ if extension.as_deref() == Some("tsv") => b'\t',
                _ => b',',
            };
            (None, parse_csv(bytes, delimiter)?)
        }
        DocKind::Docx => (None, parse_docx(bytes)?),
        DocKind::Xlsx => (None, parse_xlsx(bytes, limits)?),
        DocKind::Pptx => (None, parse_pptx(bytes)?),
        DocKind::Pdf => (None, parse_pdf(bytes, limits)?),
        DocKind::Image => {
            return Err(IngestError::Unsupported(
                "image OCR is not configured; no OCR model or engine is available".to_owned(),
            ));
        }
    };
    Ok(IngestedDocument {
        kind,
        title,
        blocks,
        trust: Trust::Untrusted,
    })
}

pub(super) fn output_size(blocks: &[LocatedBlock]) -> Result<u64, IngestError> {
    let mut total = 0_u64;
    for located in blocks {
        let block_bytes = match &located.block {
            DocBlock::Heading { text, .. } | DocBlock::Paragraph { text } => text.len(),
            DocBlock::Code { lang, text } => {
                let language_bytes = match lang {
                    Some(language) => language.len(),
                    None => 0,
                };
                language_bytes.checked_add(text.len()).ok_or_else(|| {
                    IngestError::Malformed("output byte count overflow".to_owned())
                })?
            }
            DocBlock::Table { rows } => {
                let mut bytes = 0_usize;
                for row in rows {
                    for cell in row {
                        bytes = bytes.checked_add(cell.len()).ok_or_else(|| {
                            IngestError::Malformed("output byte count overflow".to_owned())
                        })?;
                    }
                }
                bytes
            }
            DocBlock::Image { alt, .. } => match alt {
                Some(text) => text.len(),
                None => 0,
            },
        };
        let block_bytes = u64::try_from(block_bytes)
            .map_err(|_| IngestError::Malformed("output block size does not fit u64".to_owned()))?;
        total = total
            .checked_add(block_bytes)
            .ok_or_else(|| IngestError::Malformed("output byte count overflow".to_owned()))?;
    }
    Ok(total)
}

fn zip_kind(bytes: &[u8]) -> Result<DocKind, IngestError> {
    let archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| malformed("invalid ZIP container", error))?;
    let names = archive_names(&archive)?;
    if names.iter().any(|name| name == "word/document.xml") {
        return Ok(DocKind::Docx);
    }
    if names.iter().any(|name| name == "xl/workbook.xml") {
        return Ok(DocKind::Xlsx);
    }
    if names.iter().any(|name| name == "ppt/presentation.xml") {
        return Ok(DocKind::Pptx);
    }
    Err(IngestError::Unsupported(
        "ZIP container is not a supported OOXML document".to_owned(),
    ))
}

fn archive_names(archive: &ZipArchive<Cursor<&[u8]>>) -> Result<Vec<String>, IngestError> {
    archive
        .file_names()
        .map(|name| {
            name.map(|name| name.into_owned())
                .map_err(|error| malformed("invalid OOXML member name", error))
        })
        .collect()
}

fn validate_ooxml_xml_parts(bytes: &[u8], limits: IngestLimits) -> Result<(), IngestError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| malformed("invalid OOXML archive", error))?;
    let mut parts = archive_names(&archive)?
        .into_iter()
        .filter(|name| name.ends_with(".xml") || name.ends_with(".rels"))
        .collect::<Vec<_>>();
    parts.sort();
    for part in parts {
        let mut entry = archive
            .by_name(&part)
            .map_err(|error| malformed("cannot read OOXML part", error))?;
        let declared = entry.size();
        if declared > limits.max_entry_bytes {
            return Err(IngestError::TooLarge {
                bytes: declared,
                limit: limits.max_entry_bytes,
            });
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|error| malformed("cannot decompress OOXML XML part", error))?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|error| malformed("OOXML XML part is not UTF-8", error))?;
        validate_xml_bounds(source, limits)?;
    }
    Ok(())
}

fn validate_xml_bounds(source: &str, limits: IngestLimits) -> Result<(), IngestError> {
    let mut reader = XmlReader::from_str(source);
    let mut depth = 0_usize;
    let mut nodes = 0_u64;
    loop {
        match reader
            .read_event()
            .map_err(|error| malformed("invalid OOXML XML part", error))?
        {
            XmlEvent::Start(_) => {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| IngestError::Malformed("OOXML XML depth overflow".to_owned()))?;
                nodes = nodes.checked_add(1).ok_or_else(|| {
                    IngestError::Malformed("OOXML XML node count overflow".to_owned())
                })?;
            }
            XmlEvent::Empty(_) => {
                nodes = nodes.checked_add(1).ok_or_else(|| {
                    IngestError::Malformed("OOXML XML node count overflow".to_owned())
                })?
            }
            XmlEvent::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| IngestError::Malformed("unbalanced OOXML XML".to_owned()))?
            }
            XmlEvent::Eof => break,
            _ => {}
        }
        if nodes > limits.max_xml_nodes {
            return Err(IngestError::TooLarge {
                bytes: nodes,
                limit: limits.max_xml_nodes,
            });
        }
        if depth > limits.max_xml_depth {
            let actual = u64::try_from(depth).map_err(|_| {
                IngestError::Malformed("OOXML XML depth does not fit u64".to_owned())
            })?;
            let limit = u64::try_from(limits.max_xml_depth).map_err(|_| {
                IngestError::Malformed("configured XML depth does not fit u64".to_owned())
            })?;
            return Err(IngestError::TooLarge {
                bytes: actual,
                limit,
            });
        }
    }
    if depth != 0 {
        return Err(IngestError::Malformed(
            "incomplete OOXML XML part".to_owned(),
        ));
    }
    Ok(())
}

fn decode_text(bytes: &[u8]) -> Result<String, IngestError> {
    let (text, had_errors) = encoding_rs::UTF_8.decode_with_bom_removal(bytes);
    if had_errors {
        return Err(IngestError::Malformed("text is not valid UTF-8".to_owned()));
    }
    Ok(text.into_owned())
}

fn parse_text(bytes: &[u8]) -> Result<Vec<LocatedBlock>, IngestError> {
    let text = decode_text(bytes)?;
    Ok(text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| LocatedBlock {
            block: DocBlock::Paragraph {
                text: line.to_owned(),
            },
            location: SourceLocation {
                line: u32::try_from(index)
                    .ok()
                    .and_then(|value| value.checked_add(1)),
                ..SourceLocation::default()
            },
        })
        .collect())
}

fn parse_markdown(source: &str) -> Result<Vec<LocatedBlock>, IngestError> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut heading = 1_u8;
    let mut code = false;
    let mut language = None;
    let mut table = false;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut source_line = None;
    // Wikilinks are not part of ingested documents, and pulldown-cmark's wikilink extension had
    // exponential event growth on `[[a|]]` (RUSTSEC-2026-0414, fixed in 0.13.5): keep it off.
    let parser = Parser::new_ext(source, Options::all().difference(Options::ENABLE_WIKILINKS));
    for (event, span) in parser.into_offset_iter() {
        let event_line = line_for_offset(source, span.start);
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = heading_level(level);
                source_line = event_line;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                code = true;
                source_line = event_line;
                language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_owned)
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Event::Start(Tag::Table(_)) => {
                table = true;
                source_line = event_line;
            }
            Event::Start(Tag::Paragraph) => source_line = event_line,
            Event::Start(Tag::TableHead | Tag::TableRow) => row = Vec::new(),
            Event::Start(Tag::TableCell) => {
                text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let value = std::mem::take(&mut text);
                let level = std::mem::replace(&mut heading, 1);
                blocks.push(LocatedBlock {
                    block: DocBlock::Heading { level, text: value },
                    location: SourceLocation {
                        line: source_line,
                        ..SourceLocation::default()
                    },
                });
            }
            Event::End(TagEnd::Paragraph) if !table && !code => {
                push_paragraph(&mut blocks, &mut text, source_line)
            }
            Event::End(TagEnd::CodeBlock) => {
                let value = std::mem::take(&mut text);
                blocks.push(LocatedBlock {
                    block: DocBlock::Code {
                        lang: language.take(),
                        text: value,
                    },
                    location: SourceLocation {
                        line: source_line,
                        ..SourceLocation::default()
                    },
                });
                code = false;
            }
            Event::End(TagEnd::TableCell) => {
                row.push(std::mem::take(&mut text));
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if !row.is_empty() {
                    rows.push(std::mem::take(&mut row));
                }
            }
            Event::End(TagEnd::Table) => {
                if !rows.is_empty() {
                    blocks.push(LocatedBlock {
                        block: DocBlock::Table {
                            rows: std::mem::take(&mut rows),
                        },
                        location: SourceLocation {
                            line: source_line,
                            ..SourceLocation::default()
                        },
                    });
                }
                table = false;
            }
            Event::Text(value)
            | Event::Code(value)
            | Event::Html(value)
            | Event::InlineHtml(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Rule => blocks.push(LocatedBlock {
                block: DocBlock::Paragraph {
                    text: "---".to_owned(),
                },
                location: SourceLocation {
                    line: event_line,
                    ..SourceLocation::default()
                },
            }),
            Event::TaskListMarker(checked) => text.push_str(if checked { "[x] " } else { "[ ] " }),
            Event::Start(Tag::Item) => {
                source_line = event_line;
                text.push_str("• ");
            }
            Event::End(TagEnd::Item) => push_paragraph(&mut blocks, &mut text, source_line),
            _ => {}
        }
    }
    if !text.trim().is_empty() {
        push_paragraph(&mut blocks, &mut text, source_line);
    }
    Ok(blocks)
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn push_paragraph(blocks: &mut Vec<LocatedBlock>, text: &mut String, line: Option<u32>) {
    let value = std::mem::take(text);
    if !value.trim().is_empty() {
        blocks.push(LocatedBlock {
            block: DocBlock::Paragraph {
                text: value.trim().to_owned(),
            },
            location: SourceLocation {
                line,
                ..SourceLocation::default()
            },
        });
    }
}

fn line_for_offset(source: &str, offset: usize) -> Option<u32> {
    let bytes = source.as_bytes().get(..offset)?;
    let lines = bytes
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        .checked_add(1)?;
    u32::try_from(lines).ok()
}

fn parse_html(
    source: &str,
    limits: IngestLimits,
) -> Result<(Option<String>, Vec<LocatedBlock>), IngestError> {
    let converter = HtmlToMarkdown::builder()
        .scripting_enabled(false)
        .skip_tags(vec![
            "script", "style", "noscript", "template", "svg", "head",
        ])
        .build();
    let tree = converter
        .html_to_tree(source)
        .map_err(|error| malformed("cannot parse HTML DOM", error))?;
    validate_dom_bounds(&tree, limits)?;
    let title = html_title(&tree);
    strip_hidden_nodes(&tree);
    let markdown = converter.tree_to_markdown(&tree);
    let mut blocks = parse_markdown(&markdown)?;
    for block in &mut blocks {
        block.location.line = None;
    }
    Ok((title, blocks))
}

fn validate_dom_bounds(root: &Rc<Node>, limits: IngestLimits) -> Result<(), IngestError> {
    let mut pending = vec![(Rc::clone(root), 1_usize)];
    let mut visited = 0_usize;
    while let Some((node, depth)) = pending.pop() {
        visited = visited
            .checked_add(1)
            .ok_or_else(|| IngestError::Malformed("HTML DOM node count overflow".to_owned()))?;
        if visited > limits.max_dom_nodes {
            let actual = u64::try_from(visited)
                .map_err(|_| IngestError::Malformed("HTML DOM size does not fit u64".to_owned()))?;
            let limit = u64::try_from(limits.max_dom_nodes).map_err(|_| {
                IngestError::Malformed("configured HTML node limit does not fit u64".to_owned())
            })?;
            return Err(IngestError::TooLarge {
                bytes: actual,
                limit,
            });
        }
        if depth > limits.max_dom_depth {
            let actual = u64::try_from(depth).map_err(|_| {
                IngestError::Malformed("HTML DOM depth does not fit u64".to_owned())
            })?;
            let limit = u64::try_from(limits.max_dom_depth).map_err(|_| {
                IngestError::Malformed("configured HTML depth limit does not fit u64".to_owned())
            })?;
            return Err(IngestError::TooLarge {
                bytes: actual,
                limit,
            });
        }
        let next_depth = depth
            .checked_add(1)
            .ok_or_else(|| IngestError::Malformed("HTML DOM depth overflow".to_owned()))?;
        pending.extend(
            node.children
                .borrow()
                .iter()
                .map(|child| (Rc::clone(child), next_depth)),
        );
    }
    Ok(())
}

fn html_title(root: &Rc<Node>) -> Option<String> {
    let mut pending = vec![(Rc::clone(root), false)];
    let mut title = String::new();
    while let Some((node, in_title)) = pending.pop() {
        let in_title = in_title
            || matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "title");
        if in_title && let NodeData::Text { contents } = &node.data {
            title.push_str(contents.borrow().as_ref());
        }
        pending.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .map(|child| (Rc::clone(child), in_title)),
        );
    }
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_owned())
    }
}

fn strip_hidden_nodes(root: &Rc<Node>) {
    let mut pending = vec![Rc::clone(root)];
    while let Some(node) = pending.pop() {
        let children = node
            .children
            .borrow()
            .iter()
            .map(Rc::clone)
            .collect::<Vec<_>>();
        let visible = children
            .into_iter()
            .filter(|child| !is_hidden(child))
            .collect::<Vec<_>>();
        *node.children.borrow_mut() = visible.iter().map(Rc::clone).collect();
        pending.extend(visible);
    }
}

fn is_hidden(node: &Rc<Node>) -> bool {
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return false;
    };
    let tag = name.local.as_ref();
    if matches!(
        tag,
        "script"
            | "style"
            | "noscript"
            | "template"
            | "svg"
            | "head"
            | "iframe"
            | "object"
            | "embed"
    ) {
        return true;
    }
    attrs.borrow().iter().any(|attribute| {
        let key = attribute.name.local.as_ref();
        let value = attribute.value.as_ref();
        key == "hidden"
            || (key == "aria-hidden" && value.eq_ignore_ascii_case("true"))
            || (key == "type" && tag == "input" && value.eq_ignore_ascii_case("hidden"))
            || (key == "style" && style_hides_content(value))
    })
}

fn style_hides_content(style: &str) -> bool {
    let normalized = style
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    normalized.contains("display:none")
        || normalized.contains("visibility:hidden")
        || normalized.contains("opacity:0")
}

fn parse_csv(bytes: &[u8], delimiter: u8) -> Result<Vec<LocatedBlock>, IngestError> {
    let decoded = match encoding_rs::UTF_8.decode_with_bom_removal(bytes) {
        (value, false) => value.into_owned(),
        (_, true) => encoding_rs::WINDOWS_1252
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    };
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(decoded.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|error| malformed("invalid CSV", error))?;
        rows.push(record.iter().map(str::to_owned).collect::<Vec<_>>());
    }
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![LocatedBlock {
        block: DocBlock::Table { rows },
        location: SourceLocation {
            line: Some(1),
            ..SourceLocation::default()
        },
    }])
}

fn parse_docx(bytes: &[u8]) -> Result<Vec<LocatedBlock>, IngestError> {
    let options = docx_rs::ReadDocxOptions::default().with_image_previews(false);
    let docx = docx_rs::read_docx_with_options(bytes, options)
        .map_err(|error| malformed("invalid DOCX", error))?;
    let mut blocks = Vec::new();
    for child in &docx.document.children {
        match child {
            docx_rs::DocumentChild::Paragraph(paragraph) => {
                let text = paragraph_text(paragraph);
                if text.trim().is_empty() {
                    continue;
                }
                let style_level = paragraph
                    .property
                    .style
                    .as_ref()
                    .filter(|style| style.val.starts_with("Heading"))
                    .and_then(|style| style.val.chars().last())
                    .and_then(|digit| digit.to_digit(10))
                    .and_then(|level| u8::try_from(level).ok());
                let block = match style_level {
                    Some(level) => DocBlock::Heading { level, text },
                    None => DocBlock::Paragraph { text },
                };
                blocks.push(LocatedBlock {
                    block,
                    location: SourceLocation::default(),
                });
            }
            docx_rs::DocumentChild::Table(table) => {
                let rows = table
                    .rows
                    .iter()
                    .map(|row| match row {
                        docx_rs::TableChild::TableRow(row) => row
                            .cells
                            .iter()
                            .map(|cell| match cell {
                                docx_rs::TableRowChild::TableCell(cell) => cell
                                    .children
                                    .iter()
                                    .filter_map(|content| match content {
                                        docx_rs::TableCellContent::Paragraph(paragraph) => {
                                            Some(paragraph_text(paragraph))
                                        }
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" "),
                            })
                            .collect::<Vec<_>>(),
                    })
                    .collect::<Vec<_>>();
                blocks.push(LocatedBlock {
                    block: DocBlock::Table { rows },
                    location: SourceLocation::default(),
                });
            }
            _ => {}
        }
    }
    Ok(blocks)
}

fn paragraph_text(paragraph: &docx_rs::Paragraph) -> String {
    let mut text = String::new();
    for child in &paragraph.children {
        if let docx_rs::ParagraphChild::Run(run) = child {
            for run_child in &run.children {
                match run_child {
                    docx_rs::RunChild::Text(value) => text.push_str(&value.text),
                    docx_rs::RunChild::Tab(_) | docx_rs::RunChild::PTab(_) => text.push('\t'),
                    docx_rs::RunChild::Break(_) | docx_rs::RunChild::CarriageReturn(_) => {
                        text.push('\n')
                    }
                    _ => {}
                }
            }
        }
    }
    text
}

fn parse_xlsx(bytes: &[u8], limits: IngestLimits) -> Result<Vec<LocatedBlock>, IngestError> {
    validate_xlsx_layout(bytes, limits)?;
    let mut workbook: Xlsx<Cursor<&[u8]>> = open_workbook_from_rs(Cursor::new(bytes))
        .map_err(|error| malformed("invalid XLSX", error))?;
    let names = workbook.sheet_names();
    let mut blocks = Vec::new();
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| malformed("cannot read XLSX worksheet", error))?;
        let rows = range
            .rows()
            .map(|row| row.iter().map(ToString::to_string).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        if rows.is_empty() {
            continue;
        }
        blocks.push(LocatedBlock {
            block: DocBlock::Heading {
                level: 2,
                text: name.clone(),
            },
            location: SourceLocation {
                sheet: Some(name.clone()),
                ..SourceLocation::default()
            },
        });
        blocks.push(LocatedBlock {
            block: DocBlock::Table { rows },
            location: SourceLocation {
                sheet: Some(name),
                ..SourceLocation::default()
            },
        });
    }
    Ok(blocks)
}

fn validate_xlsx_layout(bytes: &[u8], limits: IngestLimits) -> Result<(), IngestError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| malformed("invalid XLSX archive", error))?;
    let mut sheet_parts = archive_names(&archive)?
        .into_iter()
        .filter(|name| name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
        .collect::<Vec<_>>();
    sheet_parts.sort();
    for part in sheet_parts {
        let mut entry = archive
            .by_name(&part)
            .map_err(|error| malformed("cannot read XLSX worksheet", error))?;
        let declared = entry.size();
        if declared > limits.max_entry_bytes {
            return Err(IngestError::TooLarge {
                bytes: declared,
                limit: limits.max_entry_bytes,
            });
        }
        let mut xml_bytes = Vec::new();
        entry
            .read_to_end(&mut xml_bytes)
            .map_err(|error| malformed("cannot decompress XLSX worksheet XML", error))?;
        let xml = std::str::from_utf8(&xml_bytes)
            .map_err(|error| malformed("XLSX worksheet XML is not UTF-8", error))?;
        validate_worksheet_xml(xml, limits)?;
    }
    Ok(())
}

fn validate_worksheet_xml(xml: &str, limits: IngestLimits) -> Result<(), IngestError> {
    let mut reader = XmlReader::from_str(xml);
    let mut depth = 0_usize;
    let mut nodes = 0_u64;
    let mut cell_count = 0_u64;
    let mut max_row = 0_u64;
    let mut max_column = 0_u64;
    loop {
        match reader
            .read_event()
            .map_err(|error| malformed("invalid XLSX worksheet XML", error))?
        {
            XmlEvent::Start(element) => {
                depth = depth.checked_add(1).ok_or_else(|| {
                    IngestError::Malformed("worksheet XML depth overflow".to_owned())
                })?;
                nodes = nodes.checked_add(1).ok_or_else(|| {
                    IngestError::Malformed("worksheet XML node count overflow".to_owned())
                })?;
                inspect_worksheet_element(
                    &element,
                    &mut cell_count,
                    &mut max_row,
                    &mut max_column,
                )?;
            }
            XmlEvent::Empty(element) => {
                nodes = nodes.checked_add(1).ok_or_else(|| {
                    IngestError::Malformed("worksheet XML node count overflow".to_owned())
                })?;
                inspect_worksheet_element(
                    &element,
                    &mut cell_count,
                    &mut max_row,
                    &mut max_column,
                )?;
            }
            XmlEvent::End(_) => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    IngestError::Malformed("unbalanced XLSX worksheet XML".to_owned())
                })?;
            }
            XmlEvent::Eof => break,
            _ => {}
        }
        if nodes > limits.max_xml_nodes {
            return Err(IngestError::TooLarge {
                bytes: nodes,
                limit: limits.max_xml_nodes,
            });
        }
        if depth > limits.max_xml_depth {
            let actual = u64::try_from(depth).map_err(|_| {
                IngestError::Malformed("worksheet XML depth does not fit u64".to_owned())
            })?;
            let limit = u64::try_from(limits.max_xml_depth).map_err(|_| {
                IngestError::Malformed("configured XML depth does not fit u64".to_owned())
            })?;
            return Err(IngestError::TooLarge {
                bytes: actual,
                limit,
            });
        }
        if cell_count > limits.max_cells {
            return Err(IngestError::TooLarge {
                bytes: cell_count,
                limit: limits.max_cells,
            });
        }
        let area = max_row
            .checked_mul(max_column)
            .ok_or_else(|| IngestError::Malformed("worksheet dimensions overflow".to_owned()))?;
        if area > limits.max_cells {
            return Err(IngestError::TooLarge {
                bytes: area,
                limit: limits.max_cells,
            });
        }
    }
    if depth != 0 {
        return Err(IngestError::Malformed(
            "incomplete XLSX worksheet XML".to_owned(),
        ));
    }
    Ok(())
}

fn inspect_worksheet_element(
    element: &quick_xml::events::BytesStart<'_>,
    cell_count: &mut u64,
    max_row: &mut u64,
    max_column: &mut u64,
) -> Result<(), IngestError> {
    let name = element.local_name();
    let local_name = name.as_ref();
    if local_name != "c" && local_name != "dimension" {
        return Ok(());
    }
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|error| malformed("invalid XLSX XML attribute", error))?;
        let key = attribute.key.as_ref();
        if key != "r" && !(local_name == "dimension" && key == "ref") {
            continue;
        }
        let value = attribute.value.as_ref();
        if local_name == "c" {
            *cell_count = cell_count.checked_add(1).ok_or_else(|| {
                IngestError::Malformed("worksheet cell count overflow".to_owned())
            })?;
            let (column, row) = parse_cell_ref(value)
                .ok_or_else(|| IngestError::Malformed("invalid XLSX cell coordinate".to_owned()))?;
            *max_row = (*max_row).max(row);
            *max_column = (*max_column).max(column);
        } else {
            let last = match value.rsplit(':').next() {
                Some(last) => last,
                None => value,
            };
            let (column, row) = parse_cell_ref(last)
                .ok_or_else(|| IngestError::Malformed("invalid XLSX dimension".to_owned()))?;
            *max_row = (*max_row).max(row);
            *max_column = (*max_column).max(column);
        }
    }
    Ok(())
}

fn parse_cell_ref(value: &str) -> Option<(u64, u64)> {
    let mut column = 0_u64;
    let mut row = 0_u64;
    let mut found_column = false;
    let mut found_row = false;
    for character in value.chars().filter(|character| *character != '$') {
        if character.is_ascii_alphabetic() && !found_row {
            let digit = u64::from(character.to_ascii_uppercase())
                .checked_sub(u64::from('A'))?
                .checked_add(1)?;
            column = column.checked_mul(26)?.checked_add(digit)?;
            found_column = true;
        } else if character.is_ascii_digit() {
            found_row = true;
            row = row
                .checked_mul(10)?
                .checked_add(u64::from(character).checked_sub(u64::from('0'))?)?;
        } else {
            return None;
        }
    }
    if found_column && found_row && row > 0 {
        Some((column, row))
    } else {
        None
    }
}

fn parse_pptx(bytes: &[u8]) -> Result<Vec<LocatedBlock>, IngestError> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| malformed("invalid PPTX", error))?;
    let mut slide_names = archive_names(&archive)?
        .into_iter()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .collect::<Vec<_>>();
    slide_names.sort_by(|left, right| {
        let number = |name: &str| {
            name.strip_prefix("ppt/slides/slide")
                .and_then(|tail| tail.strip_suffix(".xml"))
                .and_then(|value| value.parse::<u32>().ok())
        };
        number(left).cmp(&number(right))
    });
    let mut blocks = Vec::new();
    for (index, name) in slide_names.iter().enumerate() {
        let mut entry = archive
            .by_name(name)
            .map_err(|error| malformed("cannot read PPTX slide", error))?;
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|error| malformed("invalid PPTX slide encoding", error))?;
        let mut reader = XmlReader::from_str(&xml);
        reader.config_mut().trim_text(true);
        let mut paragraphs = Vec::new();
        let mut current = String::new();
        loop {
            match reader
                .read_event()
                .map_err(|error| malformed("invalid PPTX slide XML", error))?
            {
                XmlEvent::Start(event) if event.local_name().as_ref() == "p" => current.clear(),
                XmlEvent::Text(event) => current.push_str(&event.xml10_content()),
                XmlEvent::End(event) if event.local_name().as_ref() == "p" => {
                    if !current.trim().is_empty() {
                        paragraphs.push(std::mem::take(&mut current));
                    }
                }
                XmlEvent::Eof => break,
                _ => {}
            }
        }
        if !paragraphs.is_empty() {
            let page = u32::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1));
            let page_label = match page {
                Some(page) => page.to_string(),
                None => "unknown".to_owned(),
            };
            blocks.push(LocatedBlock {
                block: DocBlock::Heading {
                    level: 2,
                    text: format!("Slide {page_label}"),
                },
                location: SourceLocation {
                    page,
                    ..SourceLocation::default()
                },
            });
            blocks.extend(paragraphs.into_iter().map(|text| LocatedBlock {
                block: DocBlock::Paragraph { text },
                location: SourceLocation {
                    page,
                    ..SourceLocation::default()
                },
            }));
        }
    }
    Ok(blocks)
}

fn parse_pdf(bytes: &[u8], limits: IngestLimits) -> Result<Vec<LocatedBlock>, IngestError> {
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .map_err(|error| malformed("invalid or encrypted PDF", error))?;
    if pages.len() > limits.max_pages {
        let actual = u64::try_from(pages.len())
            .map_err(|_| IngestError::Malformed("PDF page count does not fit u64".to_owned()))?;
        let limit = u64::try_from(limits.max_pages).map_err(|_| {
            IngestError::Malformed("configured page limit does not fit u64".to_owned())
        })?;
        return Err(IngestError::TooLarge {
            bytes: actual,
            limit,
        });
    }
    let mut blocks = Vec::new();
    for (index, page) in pages.into_iter().enumerate() {
        let page_number = u32::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1));
        if page.trim().is_empty() {
            let page_label = match page_number {
                Some(page) => page.to_string(),
                None => "unknown".to_owned(),
            };
            return Err(IngestError::Unsupported(format!(
                "PDF page {page_label} has no text layer; OCR is not configured"
            )));
        }
        blocks.push(LocatedBlock {
            block: DocBlock::Paragraph {
                text: page.trim().to_owned(),
            },
            location: SourceLocation {
                page: page_number,
                ..SourceLocation::default()
            },
        });
    }
    if blocks.is_empty() {
        return Err(IngestError::Unsupported(
            "PDF has no extractable text; OCR is not configured".to_owned(),
        ));
    }
    Ok(blocks)
}

fn malformed(context: &str, error: impl std::fmt::Display) -> IngestError {
    IngestError::Malformed(format!("{context}: {error}"))
}
