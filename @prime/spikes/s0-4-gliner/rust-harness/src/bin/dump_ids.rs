//! Dump gliner2-rs input_ids for golden ids. Usage: dump_ids <tokenizer.json> <inputs.jsonl> <out.json> <id>...
use anyhow::{anyhow, Result};
use gliner2_rs::{SchemaTask, SchemaTransformer};
use serde_json::{json, Map, Value};

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 5 {
        return Err(anyhow!("usage: dump_ids <tokenizer.json> <inputs> <out.json> <id>..."));
    }
    let tr = SchemaTransformer::from_tokenizer_file(std::path::Path::new(&a[1]))?;
    let raw = std::fs::read_to_string(&a[2])?;
    let mut out = Map::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)?;
        let id = v.get("id").and_then(Value::as_str).ok_or_else(|| anyhow!("id"))?;
        if !a[4..].iter().any(|x| x == id) {
            continue;
        }
        let text = v.get("text").and_then(Value::as_str).ok_or_else(|| anyhow!("text"))?;
        let schema = v.get("schema").and_then(Value::as_object).ok_or_else(|| anyhow!("schema"))?;
        let mut tasks = Vec::new();
        for (name, labels) in schema {
            let labels: Vec<String> = labels.as_array().ok_or_else(|| anyhow!("labels"))?.iter().filter_map(|l| l.as_str().map(str::to_owned)).collect();
            tasks.push(SchemaTask::classification(name.clone(), labels));
        }
        let rec = tr.transform(text, &tasks)?;
        out.insert(id.to_owned(), json!({"input_ids": rec.input_ids}));
    }
    std::fs::write(&a[3], serde_json::to_string(&out)?)?;
    Ok(())
}
