//! S0-4 harness: runs the golden set through gliner2-rs and benchmarks latency / RSS.
//! Usage: s0-4-harness <model_dir> <fp32|fp16> <threads> <inputs.jsonl> <out.jsonl> <bench.json>
use anyhow::{anyhow, Context, Result};
use gliner2_rs::{Precision, SchemaTask, SpanConfig, SpanEngine};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};

struct Item {
    id: String,
    kind: String,
    text: String,
    tasks: Vec<SchemaTask>,
}

fn load_items(path: &str, normalize: bool) -> Result<Vec<Item>> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
    let mut items = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| anyhow!("missing {k}"));
        let schema = v.get("schema").and_then(Value::as_object).ok_or_else(|| anyhow!("missing schema"))?;
        let mut tasks = Vec::new();
        for (name, labels) in schema {
            let labels: Vec<String> = labels
                .as_array()
                .ok_or_else(|| anyhow!("labels not array"))?
                .iter()
                .filter_map(|l| l.as_str().map(str::to_owned))
                .collect();
            tasks.push(SchemaTask::classification(name.clone(), labels));
        }
        let mut text = s("text")?;
        // gliner2 (Python) 2.1.0 `_normalize_text`: text must end with . ! or ? (empty -> ".")
        if normalize && !text.ends_with(['.', '!', '?']) {
            text.push('.');
        }
        items.push(Item { id: s("id")?, kind: s("kind")?, text, tasks });
    }
    Ok(items)
}

fn rss_mb(sys: &mut System, pid: Pid) -> u64 {
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory() / (1024 * 1024)).unwrap_or(0)
}

fn pct(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len().saturating_sub(1)) as f64 * q).round() as usize;
    sorted.get(idx).copied().unwrap_or(0.0)
}

fn stats(mut ms: Vec<f64>) -> Value {
    ms.sort_by(|a, b| a.total_cmp(b));
    json!({"n": ms.len(), "p50_ms": pct(&ms, 0.5), "p95_ms": pct(&ms, 0.95),
           "min_ms": ms.first().copied().unwrap_or(0.0), "max_ms": ms.last().copied().unwrap_or(0.0)})
}

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 7 {
        return Err(anyhow!("usage: harness <model_dir> <fp32|fp16> <threads> <inputs> <out> <bench.json>"));
    }
    let precision = match a[2].as_str() {
        "fp32" => Precision::Fp32,
        "fp16" => Precision::Fp16,
        other => return Err(anyhow!("bad precision {other}")),
    };
    let threads: usize = a[3].parse()?;
    let pid = Pid::from_u32(std::process::id());

    // peak-RSS sampler (10 ms cadence)
    let peak = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (peak, stop) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut sys = System::new();
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(rss_mb(&mut sys, pid), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let mut sys = System::new();

    let normalize = a.get(7).map(String::as_str) == Some("normalize");
    let items = load_items(&a[4], normalize)?;
    gliner2_rs::init("s0-4");
    let t0 = Instant::now();
    let cfg = SpanConfig::new(&a[1]).with_precision(precision).with_intra_threads(threads);
    let mut engine = SpanEngine::new(cfg)?;
    let load_s = t0.elapsed().as_secs_f64();
    let rss_after_load = rss_mb(&mut sys, pid);
    eprintln!("loaded in {load_s:.2}s rss={rss_after_load}MB exec={:?} layout={:?}", engine.execution(), engine.classifier_layout());

    // golden pass
    let mut out = String::new();
    for it in &items {
        let res = engine.extract(&it.text, &it.tasks)?;
        let mut outputs = Map::new();
        for c in &res.classifications {
            let e = outputs.entry(c.task.clone()).or_insert_with(|| json!({"probs": {}}));
            if let Some(m) = e.get_mut("probs").and_then(Value::as_object_mut) {
                m.insert(c.label.clone(), json!(c.score));
            }
        }
        for t in &it.tasks {
            if let SchemaTask::Classifications { task, .. } = t {
                let top = res.verdict(task, 0.5).first().map(|c| c.label.clone()).unwrap_or_default();
                if let Some(e) = outputs.get_mut(task).and_then(Value::as_object_mut) {
                    e.insert("label".into(), json!(top));
                }
            }
        }
        out.push_str(&serde_json::to_string(&json!({"id": it.id, "kind": it.kind, "outputs": outputs}))?);
        out.push('\n');
    }
    std::fs::write(&a[5], out)?;

    // latency
    let mut bench = |kind: &str, n: usize| -> Result<Value> {
        let sel: Vec<&Item> = items.iter().filter(|i| i.kind == kind).collect();
        if sel.is_empty() {
            return Err(anyhow!("no items of kind {kind}"));
        }
        for it in sel.iter().take(3) {
            engine.extract(&it.text, &it.tasks)?;
        }
        let mut ms = Vec::new();
        for i in 0..n {
            let it = sel[i % sel.len()];
            let t = Instant::now();
            engine.extract(&it.text, &it.tasks)?;
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        Ok(stats(ms))
    };
    let l1 = bench("choice", 40)?;
    let l3 = bench("multi3", 30)?;
    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();
    let summary = json!({"precision": a[2], "threads": threads, "load_s": load_s, "rss_after_load_MB": rss_after_load,
        "normalize_trailing_period": normalize, "peak_rss_sampled_MB": peak.load(Ordering::Relaxed), "latency_1task": l1, "latency_3task": l3});
    std::fs::write(&a[6], serde_json::to_string_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
