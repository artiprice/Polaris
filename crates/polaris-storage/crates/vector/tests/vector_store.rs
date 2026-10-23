use std::error::Error;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use polaris_sdk::ids::{Cid, ModelId, SessionId};
use polaris_sdk::registry::{VectorChunk, VectorStore};
use polaris_storage_vector::{HnswStore, Metric, PayloadFilter, TableSpec, VectorScope};
use proptest::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn spec(name: &str) -> Result<TableSpec, Box<dyn Error>> {
    Ok(TableSpec {
        name: name.to_owned(),
        dimension: 3,
        embedder: ModelId::new("test/embedder-v1")?,
        metric: Metric::Cosine,
    })
}

fn cid(seed: u8) -> Cid {
    Cid::from_bytes([seed; 32])
}

fn chunk(id: u8, vector: Vec<f32>, payload: u8) -> VectorChunk {
    VectorChunk {
        id: cid(id),
        vector,
        payload: cid(payload),
    }
}

#[test]
fn creates_table_with_dimension_embedder_and_metric() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("metadata")?, VectorScope::Global)?;
    assert_eq!(store.count()?, 0);
    Ok(())
}

#[test]
fn invalid_names_dimensions_and_zero_cosine_vectors_are_rejected() -> TestResult {
    let dir = tempdir()?;
    let mut invalid_name = spec("../escape")?;
    assert!(HnswStore::open(dir.path(), &invalid_name, VectorScope::Global).is_err());
    invalid_name.name = "valid".to_owned();
    invalid_name.dimension = 0;
    assert!(HnswStore::open(dir.path(), &invalid_name, VectorScope::Global).is_err());

    let store = HnswStore::open(dir.path(), &spec("zero-vector")?, VectorScope::Global)?;
    assert!(store.upsert(&[chunk(1, vec![0.0, 0.0, 0.0], 11)]).is_err());
    Ok(())
}

#[test]
fn empty_search_and_zero_limit_return_no_results() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("empty")?, VectorScope::Global)?;
    assert!(store.search(&[1.0, 0.0, 0.0], 10)?.is_empty());
    store.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?;
    assert!(store.search(&[1.0, 0.0, 0.0], 0)?.is_empty());
    Ok(())
}

#[test]
fn rejects_dimension_embedder_and_metric_mismatch() -> TestResult {
    let dir = tempdir()?;
    let original = spec("metadata")?;
    let _store = HnswStore::open(dir.path(), &original, VectorScope::Global)?;

    let mut wrong_dimension = original.clone();
    wrong_dimension.dimension = 4;
    assert!(HnswStore::open(dir.path(), &wrong_dimension, VectorScope::Global).is_err());

    let mut wrong_embedder = original.clone();
    wrong_embedder.embedder = ModelId::new("test/embedder-v2")?;
    assert!(HnswStore::open(dir.path(), &wrong_embedder, VectorScope::Global).is_err());

    let mut wrong_metric = original;
    wrong_metric.metric = Metric::L2;
    assert!(HnswStore::open(dir.path(), &wrong_metric, VectorScope::Global).is_err());
    Ok(())
}

#[test]
fn rejects_dimension_mismatch_and_non_finite_vectors_atomically() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("validation")?, VectorScope::Global)?;
    let valid = chunk(1, vec![1.0, 0.0, 0.0], 11);
    assert_eq!(store.upsert(&[valid])?, 1);

    let invalid_dimension = chunk(2, vec![0.0, 1.0], 12);
    assert!(store.upsert(&[invalid_dimension]).is_err());
    let invalid_finite = chunk(3, vec![0.0, f32::INFINITY, 0.0], 13);
    assert!(store.upsert(&[invalid_finite]).is_err());

    assert_eq!(store.count()?, 1);
    let found = store.search(&[1.0, 0.0, 0.0], 10)?;
    assert_eq!(found.len(), 1);
    assert_eq!(found.first().map(|hit| hit.id), Some(cid(1)));
    Ok(())
}

#[test]
fn upsert_same_id_replaces_vector_and_payload() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("upsert")?, VectorScope::Global)?;
    assert_eq!(store.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?, 1);
    assert_eq!(store.upsert(&[chunk(1, vec![0.0, 1.0, 0.0], 12)])?, 1);

    let found = store.search(&[0.0, 1.0, 0.0], 1)?;
    let hit = found.first().ok_or("expected one result")?;
    assert_eq!(hit.id, cid(1));
    assert_eq!(hit.payload, cid(12));
    assert_eq!(store.count()?, 1);
    Ok(())
}

#[test]
fn successful_mutations_reclaim_obsolete_index_generations() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("cache-reclaim")?, VectorScope::Global)?;
    for seed in 1_u8..=4 {
        store.upsert(&[chunk(seed, vec![1.0, f32::from(seed), 0.0], seed)])?;
    }
    let index_files = std::fs::read_dir(dir.path())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "usearch")
        })
        .count();
    assert_eq!(index_files, 1);
    Ok(())
}

#[test]
fn delete_only_removes_ids_from_the_bound_scope() -> TestResult {
    let dir = tempdir()?;
    let table = spec("delete-scope")?;
    let global = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
    let session = HnswStore::open(
        dir.path(),
        &table,
        VectorScope::Session(SessionId::from_u128(3)),
    )?;
    global.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?;
    session.upsert(&[chunk(1, vec![0.0, 1.0, 0.0], 12)])?;

    assert_eq!(session.delete(&[cid(1)])?, 1);
    assert_eq!(session.count()?, 0);
    assert_eq!(global.count()?, 1);
    assert_eq!(global.delete(&[cid(99)])?, 0);
    Ok(())
}

#[test]
fn metric_scores_are_finite_and_higher_is_closer() -> TestResult {
    let dir = tempdir()?;
    let mut l2 = spec("l2-score")?;
    l2.metric = Metric::L2;
    let l2_store = HnswStore::open(dir.path(), &l2, VectorScope::Global)?;
    l2_store.upsert(&[
        chunk(1, vec![1.0, 0.0, 0.0], 11),
        chunk(2, vec![4.0, 0.0, 0.0], 12),
    ])?;
    let l2_hits = l2_store.search(&[1.0, 0.0, 0.0], 2)?;
    let l2_scores: Vec<f32> = l2_hits.iter().map(|hit| hit.score).collect();
    assert!(l2_scores.iter().all(|score| score.is_finite()));
    assert!(l2_scores.windows(2).all(|pair| {
        pair.first()
            .zip(pair.get(1))
            .is_some_and(|(previous, current)| previous >= current)
    }));
    assert_eq!(l2_hits.first().map(|hit| hit.id), Some(cid(1)));

    let mut dot = spec("dot-score")?;
    dot.metric = Metric::Dot;
    let dot_store = HnswStore::open(dir.path(), &dot, VectorScope::Global)?;
    dot_store.upsert(&[
        chunk(1, vec![1.0, 0.0, 0.0], 11),
        chunk(2, vec![4.0, 0.0, 0.0], 12),
    ])?;
    let dot_hits = dot_store.search(&[1.0, 0.0, 0.0], 2)?;
    assert!(dot_hits.iter().all(|hit| hit.score.is_finite()));
    assert_eq!(dot_hits.first().map(|hit| hit.id), Some(cid(2)));
    Ok(())
}

#[test]
fn filters_payloads_before_applying_result_limit() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(dir.path(), &spec("filter")?, VectorScope::Global)?;
    store.upsert(&[
        chunk(1, vec![1.0, 0.0, 0.0], 10),
        chunk(2, vec![0.9, 0.1, 0.0], 10),
        chunk(3, vec![0.0, 1.0, 0.0], 20),
    ])?;

    let filter = PayloadFilter::only([cid(20)]);
    let found = store.search_filtered(&[1.0, 0.0, 0.0], 1, &filter)?;
    assert_eq!(found.len(), 1);
    assert_eq!(found.first().map(|hit| hit.id), Some(cid(3)));
    Ok(())
}

#[test]
fn session_scope_never_reads_global_or_other_session_rows() -> TestResult {
    let dir = tempdir()?;
    let table = spec("scope")?;
    let global = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
    let session_a = HnswStore::open(
        dir.path(),
        &table,
        VectorScope::Session(SessionId::from_u128(1)),
    )?;
    let session_b = HnswStore::open(
        dir.path(),
        &table,
        VectorScope::Session(SessionId::from_u128(2)),
    )?;
    global.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?;
    session_a.upsert(&[chunk(2, vec![0.0, 1.0, 0.0], 12)])?;

    assert_eq!(global.count()?, 1);
    assert_eq!(session_a.count()?, 1);
    assert_eq!(session_b.count()?, 0);
    assert_eq!(global.search(&[1.0, 0.0, 0.0], 10)?.len(), 1);
    assert!(session_b.search(&[1.0, 0.0, 0.0], 10)?.is_empty());
    Ok(())
}

#[test]
fn reopen_rebuilds_missing_or_corrupt_index_from_sqlite_authority() -> TestResult {
    let dir = tempdir()?;
    let table = spec("recovery")?;
    {
        let store = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
        store.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?;
    }

    let mut cache_file = None;
    for entry in std::fs::read_dir(dir.path())? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "usearch")
        {
            cache_file = Some(entry.path());
            break;
        }
    }
    let cache_file = cache_file.ok_or("expected a persisted usearch index")?;
    std::fs::write(&cache_file, b"truncated index")?;

    let store = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
    assert_eq!(store.count()?, 1);
    assert_eq!(store.search(&[1.0, 0.0, 0.0], 1)?.len(), 1);
    let mut recovered_cache = false;
    for entry in std::fs::read_dir(dir.path())? {
        let entry = entry?;
        if entry.path() != cache_file
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "usearch")
            && entry.metadata()?.len() > 15
        {
            recovered_cache = true;
        }
    }
    assert!(recovered_cache);
    Ok(())
}

#[test]
fn failed_sqlite_commit_keeps_previous_rows_searchable() -> TestResult {
    let dir = tempdir()?;
    let table = spec("failed-commit")?;
    let store = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
    store.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?;

    let db = Connection::open(dir.path().join("vectors.sqlite3"))?;
    db.execute_batch(
        "CREATE TRIGGER reject_vector_insert BEFORE INSERT ON vector_chunks
         BEGIN SELECT RAISE(ABORT, 'forced commit failure'); END;",
    )?;
    assert!(store.upsert(&[chunk(2, vec![0.0, 1.0, 0.0], 12)]).is_err());
    assert_eq!(store.count()?, 1);
    let found = store.search(&[1.0, 0.0, 0.0], 1)?;
    assert_eq!(found.first().map(|hit| hit.id), Some(cid(1)));
    Ok(())
}

#[test]
fn sdk_vector_store_adapter_uses_the_handles_fixed_scope() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(
        dir.path(),
        &spec("sdk-adapter")?,
        VectorScope::Session(SessionId::from_u128(7)),
    )?;
    let adapter: &dyn VectorStore = &store;
    assert_eq!(adapter.upsert(&[chunk(1, vec![1.0, 0.0, 0.0], 11)])?, 1);
    assert_eq!(adapter.search(&[1.0, 0.0, 0.0], 1)?.len(), 1);
    assert_eq!(adapter.delete(&[cid(1)])?, 1);
    assert_eq!(adapter.count()?, 0);
    Ok(())
}

#[test]
fn concurrent_upserts_and_searches_preserve_committed_rows() -> TestResult {
    let dir = tempdir()?;
    let store = Arc::new(HnswStore::open(
        dir.path(),
        &spec("concurrent")?,
        VectorScope::Global,
    )?);
    let mut workers = Vec::new();
    for seed in 1_u8..=8 {
        let store = Arc::clone(&store);
        workers.push(thread::spawn(move || -> Result<(), String> {
            let vector = if seed % 2 == 0 {
                vec![0.0, 1.0, 0.0]
            } else {
                vec![1.0, 0.0, 0.0]
            };
            store
                .upsert(&[chunk(seed, vector.clone(), seed)])
                .map_err(|error| error.to_string())?;
            store
                .search(&vector, 4)
                .map_err(|error| error.to_string())?;
            Ok(())
        }));
    }
    for worker in workers {
        worker.join().map_err(|_| "worker panicked")??;
    }
    assert_eq!(store.count()?, 8);
    Ok(())
}

#[test]
fn concurrent_handles_upsert_batches_and_reopen_without_lost_rows() -> TestResult {
    let dir = tempdir()?;
    let table = spec("concurrent-handles")?;
    let first = Arc::new(HnswStore::open(dir.path(), &table, VectorScope::Global)?);
    let second = Arc::new(HnswStore::open(dir.path(), &table, VectorScope::Global)?);
    let first_worker = Arc::clone(&first);
    let first_insert = thread::spawn(move || -> Result<(), String> {
        let chunks = (1_u8..=40)
            .map(|seed| chunk(seed, vec![1.0, f32::from(seed), 0.0], seed))
            .collect::<Vec<_>>();
        first_worker
            .upsert(&chunks)
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    let second_insert = thread::spawn(move || -> Result<(), String> {
        let chunks = (41_u8..=80)
            .map(|seed| chunk(seed, vec![1.0, f32::from(seed), 0.0], seed))
            .collect::<Vec<_>>();
        second
            .upsert(&chunks)
            .map(|_| ())
            .map_err(|error| error.to_string())
    });
    first_insert.join().map_err(|_| "first writer panicked")??;
    second_insert
        .join()
        .map_err(|_| "second writer panicked")??;

    let reopened = HnswStore::open(dir.path(), &table, VectorScope::Global)?;
    assert_eq!(reopened.count()?, 80);
    assert_eq!(reopened.search(&[1.0, 40.0, 0.0], 80)?.len(), 80);
    Ok(())
}

#[test]
#[ignore = "manual native ingestion throughput characterization"]
fn benchmark_ingests_one_hundred_thousand_local_vectors() -> TestResult {
    let dir = tempdir()?;
    let store = HnswStore::open(
        dir.path(),
        &spec("ingestion-benchmark")?,
        VectorScope::Global,
    )?;
    let started = Instant::now();
    for batch_number in 0_u64..10 {
        let mut chunks = Vec::with_capacity(10_000);
        for offset in 0_u64..10_000 {
            let seed = batch_number * 10_000 + offset;
            let id = Cid::from_bytes(*blake3::hash(&seed.to_le_bytes()).as_bytes());
            chunks.push(VectorChunk {
                id,
                vector: vec![1.0, 0.1, 0.0],
                payload: id,
            });
        }
        assert_eq!(store.upsert(&chunks)?, 10_000);
    }
    let elapsed = started.elapsed();
    assert_eq!(store.count()?, 100_000);
    println!(
        "100000 vectors in {elapsed:?}; {:.0} vectors/second",
        100_000.0 / elapsed.as_secs_f64()
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_inserted_vector_is_top_hit_among_fifty_rows(seed in 1_u8..=50) {
        let dir = tempdir().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let store = HnswStore::open(
            dir.path(),
            &spec("property").map_err(|error| TestCaseError::fail(error.to_string()))?,
            VectorScope::Global,
        ).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let mut chunks = Vec::new();
        for value_seed in 1_u8..=50 {
            let vector = generated_vector(value_seed);
            chunks.push(chunk(value_seed, vector, value_seed));
        }
        store.upsert(&chunks).map_err(|error| TestCaseError::fail(error.to_string()))?;
        let query = generated_vector(seed);
        let hits = store.search(&query, 1).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(hits.first().map(|hit| hit.id), Some(cid(seed)));
    }
}

fn generated_vector(seed: u8) -> Vec<f32> {
    let x = f32::from(seed) / 50.0 - 0.5;
    let y = f32::from(seed.wrapping_mul(7) % 53) / 53.0 - 0.5;
    let z = f32::from(seed.wrapping_mul(11) % 59) / 59.0 - 0.5;
    vec![x, y, z]
}
