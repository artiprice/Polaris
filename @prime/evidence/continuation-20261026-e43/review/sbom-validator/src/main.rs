use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let schema: serde_json::Value = serde_json::from_slice(&std::fs::read(
        arguments.next().ok_or("schema path is required")?,
    )?)?;
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(
        arguments.next().ok_or("document path is required")?,
    )?)?;
    if arguments.next().is_some() {
        return Err("only schema and document paths are accepted".into());
    }
    let validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .should_validate_formats(true)
        .build(&schema)?;
    let errors = validator
        .iter_errors(&document)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(errors.join("\n").into());
    }
    // The official SPDX 2.3 schema describes this as a string without a
    // `format` keyword. Apply an independent RFC 3339 format check explicitly.
    let created = document
        .pointer("/creationInfo/created")
        .ok_or("creationInfo.created is required")?;
    let date_schema = serde_json::json!({"type": "string", "format": "date-time"});
    let date_validator = jsonschema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .should_validate_formats(true)
        .build(&date_schema)?;
    let date_errors = date_validator
        .iter_errors(created)
        .map(|error| format!("/creationInfo/created: {error}"))
        .collect::<Vec<_>>();
    if !date_errors.is_empty() {
        return Err(date_errors.join("\n").into());
    }
    writeln!(std::io::stdout().lock(), "SPDX 2.3 official schema: PASS; supplemental RFC3339 created format: PASS")?;
    Ok(())
}
