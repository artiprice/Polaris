// Unit tests of crates/polaris-ingest/src/lib.rs (`tests`); compiled into that module via #[path].
use std::{
    error::Error,
    io::{Cursor, Write},
};

use polaris_sdk::registry::{DocBlock, DocParser, DocSource, IngestError};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{DefaultDocParser, IngestLimits, archive};

fn parse(
    name: &str,
    mime: Option<&str>,
    bytes: &[u8],
) -> Result<polaris_sdk::registry::Document, IngestError> {
    DefaultDocParser::default().parse(DocSource { name, mime, bytes })
}

#[test]
fn plain_text_is_normalized_with_line_provenance() {
    let result = parse("notes.txt", None, b"first\n\nsecond\n");
    assert!(matches!(result, Ok(document) if document.blocks == vec![
        DocBlock::Paragraph { text: "first".to_owned() },
        DocBlock::Paragraph { text: "second".to_owned() },
    ]));
    let rich = DefaultDocParser::default().ingest_bytes("notes.txt", None, b"first\nsecond");
    assert!(
        matches!(rich, Ok(document) if document.blocks.get(1).is_some_and(|block| block.location.line == Some(2)))
    );
}

#[test]
fn markdown_headings_tables_and_code_are_preserved() {
    let result = parse(
        "sample.md",
        None,
        b"# Title\n\n| a | b |\n| - | - |\n| x | y |\n\n```rs\nlet x = 1;\n```\n",
    );
    assert!(
        matches!(result, Ok(document) if document.blocks.iter().any(|block| matches!(block, DocBlock::Heading { level: 1, text } if text == "Title"))
        && document.blocks.iter().any(|block| matches!(block, DocBlock::Table { rows } if rows.len() == 2))
        && document.blocks.iter().any(|block| matches!(block, DocBlock::Code { lang: Some(lang), .. } if lang == "rs")))
    );
}

#[test]
fn html_dom_and_converter_ignore_script_payload() {
    let result = DefaultDocParser::default().ingest_bytes("page.html", Some("text/html"), b"<html><head><title>Safe title</title><script>injected()</script></head><body><h1>Hello</h1><p>visible</p><script>visible</script><p>visible</p></body></html>");
    assert!(
        matches!(result, Ok(ref document) if document.title.as_deref() == Some("Safe title")
        && document.blocks.iter().any(|block| matches!(&block.block, DocBlock::Heading { text, .. } if text == "Hello"))
        && document.blocks.iter().filter(|block| matches!(&block.block, DocBlock::Paragraph { text } if text == "visible")).count() == 2
        && document.blocks.iter().all(|block| block.location.line.is_none())
        && document.blocks.iter().all(|block| !format!("{:?}", block.block).contains("injected"))),
        "{result:?}"
    );
}

#[test]
fn csv_rows_preserve_quoted_commas_and_newlines() {
    let result = parse("rows.csv", None, b"name,comment\nAda,\"hello,\nworld\"\n");
    assert!(
        matches!(result, Ok(document) if document.blocks.iter().any(|block| matches!(block, DocBlock::Table { rows } if rows.len() == 2 && rows.get(1).is_some_and(|row| row.get(1).is_some_and(|cell| cell == "hello,\nworld")))))
    );
}

#[test]
fn windows_1252_csv_is_decoded() {
    let result = parse("latin.csv", None, b"name\nAndr\xe9\n");
    assert!(
        matches!(result, Ok(document) if document.blocks.iter().any(|block| matches!(block, DocBlock::Table { rows } if rows.get(1).is_some_and(|row| row.first().is_some_and(|cell| cell == "Andr\u{00e9}")))))
    );
}

#[test]
fn tsv_uses_tabs_as_the_delimiter() {
    let result = parse("rows.tsv", None, b"name\tvalue\nAda\t42\n");
    assert!(
        matches!(result, Ok(document) if document.blocks.iter().any(|block| matches!(block, DocBlock::Table { rows } if rows.len() == 2 && rows.get(1).is_some_and(|row| row == &["Ada", "42"]))))
    );
}

#[test]
fn byte_signature_takes_precedence_over_misleading_pdf_extension() {
    assert!(matches!(
        DefaultDocParser::default().ingest_bytes("misleading.pdf", None, b"plain text"),
        Err(IngestError::Malformed(_))
    ));
}

#[test]
fn unknown_bytes_return_typed_unsupported() {
    let result = parse("payload.unknown", None, b"\x00\xff\x01");
    assert!(matches!(result, Err(IngestError::Unsupported(_))));
}

#[test]
fn oversize_input_rejected_before_format_parser() {
    let parser = DefaultDocParser {
        limits: IngestLimits {
            max_bytes: 2,
            ..IngestLimits::default()
        },
    };
    let result = parser.parse(DocSource {
        name: "x.txt",
        mime: None,
        bytes: b"abc",
    });
    assert!(matches!(
        result,
        Err(IngestError::TooLarge { bytes: 3, limit: 2 })
    ));
}

#[test]
fn normalized_output_and_html_dom_limits_are_enforced() {
    let parser = DefaultDocParser {
        limits: IngestLimits {
            max_output_bytes: 2,
            ..IngestLimits::default()
        },
    };
    assert!(matches!(
        parser.ingest_bytes("large.txt", None, b"abc"),
        Err(IngestError::TooLarge { bytes: 3, limit: 2 })
    ));

    let parser = DefaultDocParser {
        limits: IngestLimits {
            max_dom_nodes: 1,
            ..IngestLimits::default()
        },
    };
    assert!(matches!(
        parser.ingest_bytes("page.html", None, b"<p>text</p>"),
        Err(IngestError::TooLarge { .. })
    ));
}

#[test]
fn normal_archive_passes_and_hostile_archives_are_rejected() -> Result<(), Box<dyn Error>> {
    let limits = IngestLimits::default();
    let normal = zip_bytes(&[("word/document.xml", b"<doc/>".as_slice())])?;
    assert!(archive::validate(&normal, limits).is_ok());

    let many = zip_bytes(&[("one", b"1".as_slice()), ("two", b"2".as_slice())])?;
    assert!(
        archive::validate(
            &many,
            IngestLimits {
                max_archive_entries: 1,
                ..limits
            }
        )
        .is_err()
    );

    let traversal = zip_bytes(&[("../escape", b"x".as_slice())])?;
    assert!(archive::validate(&traversal, limits).is_err());

    let mut invalid_name = zip_bytes(&[("nullpath", b"x".as_slice())])?;
    let name_offsets = invalid_name
        .windows(8)
        .enumerate()
        .filter_map(|(offset, name)| (name == b"nullpath").then_some(offset))
        .collect::<Vec<_>>();
    assert_eq!(
        name_offsets.len(),
        2,
        "fixture must corrupt local and central names"
    );
    for offset in name_offsets {
        let end = offset.checked_add(8).ok_or("fixture offset overflow")?;
        invalid_name
            .get_mut(offset..end)
            .ok_or("fixture name is out of bounds")?
            .copy_from_slice(b"nul\0path");
    }
    assert!(matches!(
        archive::validate(&invalid_name, limits),
        Err(IngestError::Malformed(_))
    ));

    let duplicate = zip_bytes(&[
        ("a/./b", b"first".as_slice()),
        ("a/b", b"second".as_slice()),
    ])?;
    assert!(archive::validate(&duplicate, limits).is_err());

    let expanded = zip_bytes(&[("large", vec![b'x'; 100_000].as_slice())])?;
    assert!(
        archive::validate(
            &expanded,
            IngestLimits {
                max_uncompressed_bytes: 1_024,
                max_compression_ratio: u64::MAX,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        archive::validate(
            &expanded,
            IngestLimits {
                max_compression_ratio: 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(archive::validate(b"not zip", limits).is_err());
    Ok(())
}

#[test]
fn generated_docx_is_parsed_without_fabricated_line_provenance() {
    let docx = docx_rs::Docx::new().add_paragraph(
        docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text("generated DOCX")),
    );
    let mut bytes = Cursor::new(Vec::new());
    let packed = docx.pack(&mut bytes);
    assert!(packed.is_ok());
    let parsed = DefaultDocParser::default().ingest_bytes("fixture.docx", None, bytes.get_ref());
    assert!(
        matches!(parsed, Ok(document) if document.blocks.iter().any(|block| matches!(&block.block, DocBlock::Paragraph { text } if text == "generated DOCX") && block.location.line.is_none()))
    );
}

#[test]
fn generated_xlsx_and_pptx_are_parsed_by_their_real_readers() -> Result<(), Box<dyn Error>> {
    let xlsx = zip_bytes(&[
        ("[Content_Types].xml", b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>".as_slice()),
        ("_rels/.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>".as_slice()),
        ("xl/workbook.xml", b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>".as_slice()),
        ("xl/_rels/workbook.xml.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>".as_slice()),
        ("xl/worksheets/sheet1.xml", b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>header</t></is></c></row><row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>value</t></is></c></row></sheetData></worksheet>".as_slice()),
    ])?;
    let parsed = DefaultDocParser::default().ingest_bytes("fixture.xlsx", None, &xlsx);
    assert!(
        matches!(parsed, Ok(document) if document.kind == crate::DocKind::Xlsx && document.blocks.iter().any(|block| block.location.sheet.as_deref() == Some("Data")))
    );

    let far_cell_xlsx = zip_bytes(&[
        ("[Content_Types].xml", b"<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"/>".as_slice()),
        ("_rels/.rels", b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".as_slice()),
        ("xl/workbook.xml", b"<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"/>".as_slice()),
        ("xl/worksheets/sheet1.xml", b"<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><dimension ref=\"A1:XFD1048576\"/><sheetData/></worksheet>".as_slice()),
    ])?;
    let parser = DefaultDocParser {
        limits: IngestLimits {
            max_cells: 10_000,
            ..IngestLimits::default()
        },
    };
    assert!(matches!(
        parser.ingest_bytes("far-cell.xlsx", None, &far_cell_xlsx),
        Err(IngestError::TooLarge { .. })
    ));
    let parser = DefaultDocParser {
        limits: IngestLimits {
            max_xml_nodes: 1,
            ..IngestLimits::default()
        },
    };
    assert!(matches!(
        parser.ingest_bytes("fixture.xlsx", None, &xlsx),
        Err(IngestError::TooLarge { .. })
    ));

    let pptx = zip_bytes(&[
        ("ppt/presentation.xml", b"<p:presentation xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"/>".as_slice()),
        ("ppt/slides/slide1.xml", b"<p:sld xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:p><a:r><a:t>slide text</a:t></a:r></a:p></p:sld>".as_slice()),
    ])?;
    let parsed = DefaultDocParser::default().ingest_bytes("fixture.pptx", None, &pptx);
    assert!(
        matches!(parsed, Ok(document) if document.kind == crate::DocKind::Pptx && document.blocks.iter().any(|block| matches!(&block.block, DocBlock::Paragraph { text } if text == "slide text")))
    );
    Ok(())
}

#[test]
fn non_integer_compression_ratio_is_rejected_exactly() -> Result<(), Box<dyn Error>> {
    let mut state = 0x1234_5678_u32;
    let mut payload = Vec::new();
    for _ in 0..8192 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        payload.push(u8::try_from(state & 0x0f)?);
    }
    let bytes = zip_bytes(&[("ratio", &payload)])?;
    let mut archive = ZipArchive::new(Cursor::new(&bytes))?;
    let entry = archive.by_name("ratio")?;
    let compressed = entry.compressed_size();
    let declared = entry.size();
    assert!(compressed > 0);
    assert!(declared > compressed && declared < compressed.saturating_mul(2));
    drop(entry);
    assert!(
        archive::validate(
            &bytes,
            IngestLimits {
                max_compression_ratio: 1,
                ..IngestLimits::default()
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn image_and_scanned_pdf_never_claim_ocr_without_configuration() {
    let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR";
    assert!(
        matches!(DefaultDocParser::default().ingest_bytes("scan.png", None, png), Err(IngestError::Unsupported(detail)) if detail.contains("OCR is not configured"))
    );
    let scanned_pdf = minimal_pdf(None);
    assert!(
        matches!(DefaultDocParser::default().ingest_bytes("scan.pdf", None, &scanned_pdf), Err(IngestError::Unsupported(detail)) if detail.contains("OCR is not configured"))
    );
    assert!(matches!(
        DefaultDocParser::default().ingest_bytes("damaged.pdf", None, b"%PDF-1.4\n%%EOF"),
        Err(IngestError::Malformed(_))
    ));
}

#[test]
fn generated_text_pdf_is_parsed_page_by_page() {
    let bytes = minimal_pdf(Some("generated PDF text"));
    let parsed = DefaultDocParser::default().ingest_bytes("fixture.pdf", None, &bytes);
    assert!(
        matches!(parsed, Ok(document) if document.kind == crate::DocKind::Pdf
        && document.blocks.iter().any(|block| block.location.page == Some(1)
            && matches!(&block.block, DocBlock::Paragraph { text } if text.contains("generated PDF text"))))
    );
}

#[test]
fn markdown_renderer_escapes_table_separators_and_newlines() {
    let document = polaris_sdk::registry::Document {
        title: None,
        blocks: vec![DocBlock::Table {
            rows: vec![vec!["a|b".to_owned(), "x\ny".to_owned()]],
        }],
    };
    assert_eq!(
        crate::to_markdown(&document),
        "| a\\|b | x y |\n| --- | --- |\n\n"
    );
}

#[test]
fn markdown_renderer_uses_safe_dynamic_code_fences() {
    let document = polaris_sdk::registry::Document {
        title: None,
        blocks: vec![DocBlock::Code {
            lang: Some("rust\nunsafe-info".to_owned()),
            text: "let sample = ```;\n".to_owned(),
        }],
    };
    assert_eq!(
        crate::to_markdown(&document),
        "````rust\nlet sample = ```;\n````\n\n"
    );
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer.start_file(
            name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )?;
        writer.write_all(bytes)?;
    }
    Ok(writer.finish()?.into_inner())
}

fn minimal_pdf(text: Option<&str>) -> Vec<u8> {
    let content = match text {
        Some(text) => format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET"),
        None => String::new(),
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!(
            "{} 0 obj\n{}\nendobj\n",
            index.saturating_add(1),
            object
        ));
    }
    let xref_offset = pdf.len();
    pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF"
    ));
    pdf.into_bytes()
}

/// RUSTSEC-2026-0414: `[[a|]]` wikilinks made pulldown-cmark's event count double per link. Ingest
/// keeps wikilinks off, so a document of many such links parses in linear time as plain text.
#[test]
fn wikilink_bombs_parse_as_plain_text() -> Result<(), Box<dyn std::error::Error>> {
    let bomb = "[[a|]] ".repeat(64);
    let started = std::time::Instant::now();
    let parsed = DefaultDocParser::default().ingest_bytes("bomb.md", None, bomb.as_bytes())?;
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    let text: String = parsed
        .blocks
        .iter()
        .map(|block| format!("{block:?}"))
        .collect();
    assert!(text.contains("[[a|]]"), "{text}");
    Ok(())
}
