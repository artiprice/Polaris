use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::error::McpError;

const MAX_SCHEMA_BYTES: usize = 262_144;
const MAX_SCHEMA_DEPTH: usize = 64;
const MAX_SCHEMA_NODES: usize = 10_000;

/// A JSON schema transformed for the bounded tool-calling registry.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedSchema {
    /// Supported schema keywords, validated by the JSON Schema implementation.
    pub schema: Value,
    /// Sorted unique names of keywords omitted for the registry dialect.
    pub dropped_keywords: Vec<String>,
}

/// Validates and normalizes an MCP tool input schema without fetching remote references.
///
/// The normalizer bounds input size and structure, rejects `$ref` because the downstream tool
/// schema dialect cannot represent reference semantics safely, and records every dropped keyword.
///
/// # Errors
/// Returns [`McpError::SchemaInvalid`] for malformed, oversized, cyclic/reference-bearing, or
/// otherwise unsupported schemas.
pub fn normalize_schema(schema: &Value) -> Result<NormalizedSchema, McpError> {
    let source_bytes = serde_json::to_vec(schema).map_err(|_| McpError::SchemaInvalid)?;
    if source_bytes.len() > MAX_SCHEMA_BYTES || !schema.is_object() {
        return Err(McpError::SchemaInvalid);
    }
    let mut node_count = 0_usize;
    preflight_tree(schema, 0, &mut node_count)?;
    reject_schema_refs(schema)?;
    let mut schema_for_validation = schema.clone();
    if let Value::Object(object) = &mut schema_for_validation {
        object.remove("$schema");
    }
    let _validator =
        jsonschema::validator_for(&schema_for_validation).map_err(|_| McpError::SchemaInvalid)?;
    let mut normalized = schema.clone();
    let mut dropped_keywords = BTreeSet::new();
    let mut normalized_nodes = 0_usize;
    normalize_schema_node(
        &mut normalized,
        0,
        &mut normalized_nodes,
        &mut dropped_keywords,
    )?;
    let output_bytes = serde_json::to_vec(&normalized).map_err(|_| McpError::SchemaInvalid)?;
    if output_bytes.len() > MAX_SCHEMA_BYTES {
        return Err(McpError::SchemaInvalid);
    }
    let dropped_keywords = dropped_keywords.into_iter().collect();
    Ok(NormalizedSchema {
        schema: normalized,
        dropped_keywords,
    })
}

fn preflight_tree(value: &Value, depth: usize, node_count: &mut usize) -> Result<(), McpError> {
    if depth > MAX_SCHEMA_DEPTH || *node_count >= MAX_SCHEMA_NODES {
        return Err(McpError::SchemaInvalid);
    }
    *node_count = node_count.checked_add(1).ok_or(McpError::SchemaInvalid)?;
    match value {
        Value::Object(object) => {
            let nested_depth = depth.checked_add(1).ok_or(McpError::SchemaInvalid)?;
            for nested in object.values() {
                preflight_tree(nested, nested_depth, node_count)?;
            }
            Ok(())
        }
        Value::Array(values) => {
            let nested_depth = depth.checked_add(1).ok_or(McpError::SchemaInvalid)?;
            for nested in values {
                preflight_tree(nested, nested_depth, node_count)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

fn reject_schema_refs(schema: &Value) -> Result<(), McpError> {
    let Value::Object(object) = schema else {
        return Err(McpError::SchemaInvalid);
    };
    if object.contains_key("$ref") {
        return Err(McpError::SchemaInvalid);
    }
    for keyword in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(Value::Object(children)) = object.get(keyword) {
            for child in children.values() {
                reject_schema_or_schema_array_refs(child)?;
            }
        }
    }
    for keyword in [
        "items",
        "additionalProperties",
        "contains",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
        "unevaluatedItems",
        "unevaluatedProperties",
    ] {
        if let Some(child) = object.get(keyword) {
            reject_schema_or_schema_array_refs(child)?;
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(Value::Array(children)) = object.get(keyword) {
            for child in children {
                reject_schema_or_schema_array_refs(child)?;
            }
        }
    }
    Ok(())
}

fn reject_schema_or_schema_array_refs(value: &Value) -> Result<(), McpError> {
    match value {
        Value::Object(_) => reject_schema_refs(value),
        Value::Array(children) => {
            for child in children {
                reject_schema_or_schema_array_refs(child)?;
            }
            Ok(())
        }
        Value::Bool(_) => Ok(()),
        Value::Null | Value::Number(_) | Value::String(_) => Err(McpError::SchemaInvalid),
    }
}

fn normalize_schema_node(
    value: &mut Value,
    depth: usize,
    node_count: &mut usize,
    dropped_keywords: &mut BTreeSet<String>,
) -> Result<(), McpError> {
    if depth > MAX_SCHEMA_DEPTH || *node_count >= MAX_SCHEMA_NODES {
        return Err(McpError::SchemaInvalid);
    }
    *node_count = node_count.checked_add(1).ok_or(McpError::SchemaInvalid)?;
    match value {
        Value::Object(object) => normalize_object(object, depth, node_count, dropped_keywords),
        Value::Bool(_) => Ok(()),
        Value::Array(_) | Value::Null | Value::Number(_) | Value::String(_) => {
            Err(McpError::SchemaInvalid)
        }
    }
}

fn normalize_object(
    object: &mut Map<String, Value>,
    depth: usize,
    node_count: &mut usize,
    dropped_keywords: &mut BTreeSet<String>,
) -> Result<(), McpError> {
    let supported = [
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "const",
        "default",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "minProperties",
        "maxProperties",
        "anyOf",
        "oneOf",
        "allOf",
        "not",
    ];
    if object.contains_key("$ref") {
        // `$ref` is rejected explicitly rather than silently losing its semantics.
        return Err(McpError::SchemaInvalid);
    }
    let annotations = [
        "$schema",
        "$id",
        "$comment",
        "title",
        "description",
        "default",
        "examples",
        "deprecated",
        "readOnly",
        "writeOnly",
    ];
    let unsupported: Vec<String> = object
        .keys()
        .filter(|key| !supported.contains(&key.as_str()) && !annotations.contains(&key.as_str()))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        return Err(McpError::SchemaInvalid);
    }
    for keyword in annotations {
        if object.remove(keyword).is_some() {
            dropped_keywords.insert(keyword.to_owned());
        }
    }
    let nested_depth = depth.checked_add(1).ok_or(McpError::SchemaInvalid)?;
    for keyword in ["properties"] {
        if let Some(Value::Object(children)) = object.get_mut(keyword) {
            for child in children.values_mut() {
                normalize_schema_node(child, nested_depth, node_count, dropped_keywords)?;
            }
        }
    }
    for keyword in ["items", "additionalProperties", "not"] {
        if let Some(child @ Value::Object(_)) = object.get_mut(keyword) {
            normalize_schema_node(child, nested_depth, node_count, dropped_keywords)?;
        }
    }
    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(children)) = object.get_mut(keyword) {
            for child in children {
                normalize_schema_node(child, nested_depth, node_count, dropped_keywords)?;
            }
        }
    }
    Ok(())
}
