#![forbid(unsafe_code)]

use polaris_ext_mcp::{McpError, NamespacedToolId, ServerName, normalize_schema};
use serde_json::json;

#[test]
fn valid_schema_passes() {
    let schema = json!({
        "type": "object",
        "properties": {"query": {"type": "string"}},
        "required": ["query"],
        "additionalProperties": false
    });
    let result = normalize_schema(&schema);
    assert!(result.is_ok(), "valid schema should normalize");
}

#[test]
fn invalid_schema_rejected() {
    let schema = json!({"type": 42});
    let result = normalize_schema(&schema);
    assert!(matches!(result, Err(McpError::SchemaInvalid)));
}

#[test]
fn unsupported_assertion_keywords_are_rejected() {
    let schema = json!({
        "type": "object",
        "unevaluatedProperties": false,
        "properties": {"name": {"type": "string", "format": "hostname"}}
    });
    let result = normalize_schema(&schema);
    assert!(matches!(result, Err(McpError::SchemaInvalid)));
}

#[test]
fn property_names_and_nested_schemas_are_preserved() {
    let schema = json!({
        "type": "object",
        "properties": {
            "text": {"type": "string"},
            "count": {"type": "integer", "minimum": 2}
        },
        "required": ["text"]
    });
    let normalized = normalize_schema(&schema);
    assert!(normalized.is_ok());
    if let Ok(normalized) = normalized {
        assert_eq!(normalized.schema["properties"]["text"]["type"], "string");
        assert_eq!(normalized.schema["properties"]["count"]["minimum"], 2);
        assert_eq!(normalized.schema["required"], json!(["text"]));
    }
}

#[test]
fn literal_default_enum_and_const_objects_are_not_traversed_as_schemas() {
    let schema = json!({
        "type": "object",
        "properties": {
            "literal": {
                "default": {"$ref": "literal-data"},
                "enum": [{"type": "payload"}],
                "const": {"properties": {"kept": true}}
            }
        }
    });
    let normalized = normalize_schema(&schema);
    assert!(normalized.is_ok());
    if let Ok(normalized) = normalized {
        assert_eq!(
            normalized.schema["properties"]["literal"]["enum"],
            json!([{"type": "payload"}])
        );
        assert_eq!(
            normalized.schema["properties"]["literal"]["const"],
            json!({"properties": {"kept": true}})
        );
        assert!(
            normalized.schema["properties"]["literal"]
                .get("default")
                .is_none()
        );
    }
}

#[test]
fn harmless_annotation_keywords_are_dropped_and_recorded() {
    let schema = json!({"type": "string", "description": "text", "default": {"anything": true}});
    let normalized = normalize_schema(&schema);
    assert!(normalized.is_ok());
    if let Ok(normalized) = normalized {
        assert!(normalized.schema.get("description").is_none());
        assert!(normalized.schema.get("default").is_none());
        assert!(
            normalized
                .dropped_keywords
                .contains(&"description".to_owned())
        );
        assert!(normalized.dropped_keywords.contains(&"default".to_owned()));
    }
}

#[test]
fn namespaced_ids_prevent_server_collisions() {
    let first_server = ServerName::new("first-server");
    let second_server = ServerName::new("second-server");
    assert!(first_server.is_ok());
    assert!(second_server.is_ok());
    if let (Ok(first_server), Ok(second_server)) = (first_server, second_server) {
        let first = NamespacedToolId::new(first_server, "read");
        let second = NamespacedToolId::new(second_server, "read");
        assert!(first.is_ok());
        assert!(second.is_ok());
        if let (Ok(first), Ok(second)) = (first, second) {
            assert_ne!(first, second);
            assert_eq!(first.to_string(), "first-server::read");
            assert_eq!(second.to_string(), "second-server::read");
        }
    }
}

#[test]
fn ref_cycles_rejected() {
    let schema = json!({
        "$defs": {"loop": {"$ref": "#/$defs/loop"}},
        "$ref": "#/$defs/loop"
    });
    let result = normalize_schema(&schema);
    assert!(matches!(result, Err(McpError::SchemaInvalid)));
}
