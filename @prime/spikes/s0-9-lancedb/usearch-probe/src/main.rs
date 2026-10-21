//! S0-9 fallback: usearch HNSW index + SQLite metadata, 10k x 384 (same workload as the LanceDB spike).
//! Usage: `s0-9-usearch-probe [ROWS] [DIM]`.

use std::error::Error;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

type AnyResult<T> = Result<T, Box<dyn Error>>;

fn vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..dim)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 10_000) as f32 / 10_000.0 - 0.5
        })
        .collect()
}

fn main() -> AnyResult<()> {
    let mut args = std::env::args().skip(1);
    let rows: usize = args.next().map_or(Ok(10_000), |a| a.parse())?;
    let dim: usize = args.next().map_or(Ok(384), |a| a.parse())?;

    let t = Instant::now();
    let db = Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE chunk (key INTEGER PRIMARY KEY, source TEXT NOT NULL, body TEXT NOT NULL);")?;
    let tx = db.unchecked_transaction()?;
    {
        let mut ins = tx.prepare("INSERT INTO chunk (key, source, body) VALUES (?1, ?2, ?3)")?;
        for k in 0..rows {
            ins.execute((k as i64, format!("file{}.rs", k % 400), format!("chunk {k}")))?;
        }
    }
    tx.commit()?;
    println!("sqlite metadata ({rows} rows): {} ms", t.elapsed().as_millis());

    let options = IndexOptions {
        dimensions: dim,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        ..Default::default()
    };
    let index = Index::new(&options)?;
    index.reserve(rows)?;
    let t = Instant::now();
    for k in 0..rows {
        index.add(k as u64, &vector(k as u64, dim))?;
    }
    println!("usearch add ({rows} x {dim}, HNSW): {} ms; size={}", t.elapsed().as_millis(), index.size());

    let mut lat: Vec<Duration> = Vec::new();
    let mut hits = 0usize;
    let mut sql = db.prepare("SELECT source FROM chunk WHERE key = ?1")?;
    for q in 0..50u64 {
        let query = vector(q * 7 + 3, dim);
        let t = Instant::now();
        let found = index.search(&query, 10)?;
        for key in &found.keys {
            let _source: String = sql.query_row([*key as i64], |r| r.get(0))?;
            hits += 1;
        }
        lat.push(t.elapsed());
    }
    lat.sort_unstable();
    let p = |q: usize| lat.get((lat.len() - 1) * q / 100).map_or(0, |d| d.as_micros());
    println!("knn hnsw + sqlite lookup (k=10, 50 queries): p50={} us p95={} us max={} us, rows returned={hits}", p(50), p(95), p(100));
    println!("RESULT: PASS");
    Ok(())
}
