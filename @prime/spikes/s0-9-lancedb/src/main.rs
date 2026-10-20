//! Spike S0-9: LanceDB 0.40 embedded on Windows — create, append, k-NN query latency.
//! Usage: `s0-9-lancedb [ROWS] [DIM]` (defaults 10000 x 384, like a small embedding store).

use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::types::Float32Type;
use arrow_array::{FixedSizeListArray, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

type AnyResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Deterministic pseudo-random vectors (xorshift) so runs are comparable.
fn vectors(rows: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed.max(1);
    let mut out = Vec::with_capacity(rows);
    for _ in 0..rows {
        let mut v = Vec::with_capacity(dim);
        for _ in 0..dim {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let unit = f64::from(u32::try_from(s >> 40).unwrap_or(0)) / f64::from(1_u32 << 24);
            v.push((unit as f32) * 2.0 - 1.0);
        }
        out.push(v);
    }
    out
}

fn batch(schema: &Arc<Schema>, start: i32, vecs: &[Vec<f32>], dim: i32) -> AnyResult<RecordBatch> {
    let n = i32::try_from(vecs.len())?;
    let ids = Int32Array::from_iter_values(start..start + n);
    let texts = StringArray::from_iter_values((start..start + n).map(|i| format!("chunk-{i}")));
    let list = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
        vecs.iter().map(|v| Some(v.iter().copied().map(Some).collect::<Vec<_>>())),
        dim,
    );
    Ok(RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(texts), Arc::new(list)])?)
}

#[tokio::main]
async fn main() -> AnyResult<()> {
    let mut args = std::env::args().skip(1);
    let rows: usize = args.next().map_or(Ok(10_000), |a| a.parse())?;
    let dim: usize = args.next().map_or(Ok(384), |a| a.parse())?;
    let dim_i32 = i32::try_from(dim)?;

    let dir = std::env::temp_dir().join(format!("polaris-s0-9-{}", std::process::id()));
    let uri = dir.to_string_lossy().to_string();

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim_i32),
            true,
        ),
    ]));

    let t = Instant::now();
    let db = lancedb::connect(&uri).execute().await?;
    println!("connect: {} ms ({uri})", t.elapsed().as_millis());

    let half = rows / 2;
    let first = vectors(half, dim, 42);
    let second = vectors(rows - half, dim, 4242);

    let t = Instant::now();
    let table = db.create_table("chunks", batch(&schema, 0, &first, dim_i32)?).execute().await?;
    println!("create_table ({half} rows x {dim}): {} ms", t.elapsed().as_millis());

    let t = Instant::now();
    table.add(batch(&schema, i32::try_from(half)?, &second, dim_i32)?).execute().await?;
    println!("add ({} rows): {} ms", rows - half, t.elapsed().as_millis());
    println!("count_rows: {}", table.count_rows(None).await?);

    let queries = vectors(50, dim, 7);
    let mut lat = Vec::with_capacity(queries.len());
    let mut hits = 0usize;
    for q in &queries {
        let t = Instant::now();
        let results: Vec<RecordBatch> = table.query().nearest_to(q.as_slice())?.limit(10).execute().await?.try_collect().await?;
        lat.push(t.elapsed().as_micros());
        hits += results.iter().map(RecordBatch::num_rows).sum::<usize>();
    }
    lat.sort_unstable();
    let p = |q: usize| lat.get(lat.len().saturating_sub(1) * q / 100).copied().unwrap_or(0);
    println!("knn flat (k=10, {} queries): p50={} us p95={} us max={} us, rows returned={hits}", lat.len(), p(50), p(95), p(100));

    drop(table);
    drop(db);
    std::fs::remove_dir_all(&dir)?;
    println!("RESULT: PASS");
    Ok(())
}
