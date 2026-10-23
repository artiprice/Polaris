use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{VectorChunk, VectorError, VectorHit, VectorStore};
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::VectorStoreError;
use crate::types::{Metric, PayloadFilter, TableSpec, VectorScope};

/// Maximum rows accepted in one atomic write operation.
pub const MAX_BATCH_SIZE: usize = 10_000;

const DATABASE_FILE: &str = "vectors.sqlite3";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);
static ENCODING_POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
const MAX_ENCODING_THREADS: usize = 4;

/// A vector handle permanently bound to a table and an access scope.
pub struct HnswStore {
    inner: Mutex<StoreInner>,
}

struct StoreInner {
    connection: Connection,
    index: Index,
    rows: HashMap<u64, StoredRow>,
    id_to_key: HashMap<Cid, u64>,
    table: TableSpec,
    scope_key: Vec<u8>,
    cache_key: String,
    root: PathBuf,
    generation: i64,
    cache_error: Option<String>,
}

#[derive(Clone)]
struct StoredRow {
    id: Cid,
    payload: Cid,
}

struct RawRow {
    row_id: i64,
    id: Vec<u8>,
    payload: Vec<u8>,
    vector: Vec<u8>,
}

impl HnswStore {
    /// Opens or creates a table partition under `root`.
    ///
    /// A handle can access only the supplied table and scope. Existing table metadata must match
    /// exactly. Missing, stale, or corrupt HNSW cache files are reconstructed from SQLite.
    ///
    /// # Errors
    /// Returns a configuration, SQLite, filesystem, or index error.
    pub fn open(
        root: &Path,
        table: &TableSpec,
        scope: VectorScope,
    ) -> Result<Self, VectorStoreError> {
        table.validate()?;
        let dimension = i64::try_from(table.dimension).map_err(|_| {
            VectorStoreError::InvalidConfiguration(
                "dimension does not fit SQLite INTEGER".to_owned(),
            )
        })?;
        fs::create_dir_all(root)?;
        let database_path = root.join(DATABASE_FILE);
        let mut connection = Connection::open(database_path)?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let journal_mode: String =
            connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(VectorStoreError::InvalidConfiguration(format!(
                "SQLite WAL is unavailable (mode {journal_mode:?})"
            )));
        }
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS vector_tables (
                name TEXT PRIMARY KEY NOT NULL,
                dimension INTEGER NOT NULL CHECK (dimension > 0),
                embedder TEXT NOT NULL,
                metric TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS vector_partitions (
                table_name TEXT NOT NULL REFERENCES vector_tables(name) ON DELETE CASCADE,
                scope BLOB NOT NULL,
                generation INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
                indexed_generation INTEGER NOT NULL DEFAULT -1,
                index_file TEXT,
                PRIMARY KEY (table_name, scope)
            );
            CREATE TABLE IF NOT EXISTS vector_chunks (
                row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                table_name TEXT NOT NULL,
                scope BLOB NOT NULL,
                id BLOB NOT NULL CHECK (length(id) = 32),
                payload BLOB NOT NULL CHECK (length(payload) = 32),
                vector BLOB NOT NULL,
                UNIQUE (table_name, scope, id),
                FOREIGN KEY (table_name, scope)
                    REFERENCES vector_partitions(table_name, scope) ON DELETE CASCADE
            );",
        )?;

        let scope_key = scope.key();
        {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let stored: Option<(i64, String, String)> = tx
                .query_row(
                    "SELECT dimension, embedder, metric FROM vector_tables WHERE name = ?1",
                    [table.name.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            match stored {
                Some((stored_dimension, embedder, metric))
                    if stored_dimension == dimension
                        && embedder == table.embedder.as_str()
                        && metric == table.metric.as_str() => {}
                Some((stored_dimension, embedder, metric)) => {
                    return Err(VectorStoreError::TableMismatch(format!(
                        "table {:?} is dimension {stored_dimension}, embedder {embedder:?}, metric {metric:?}",
                        table.name
                    )));
                }
                None => {
                    tx.execute(
                        "INSERT INTO vector_tables(name, dimension, embedder, metric)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            table.name,
                            dimension,
                            table.embedder.as_str(),
                            table.metric.as_str()
                        ],
                    )?;
                }
            }
            tx.execute(
                "INSERT OR IGNORE INTO vector_partitions(table_name, scope) VALUES (?1, ?2)",
                params![table.name, scope_key.as_slice()],
            )?;
            tx.commit()?;
        }

        let cache_key = cache_key(&table.name, &scope_key);
        let mut inner = StoreInner {
            connection,
            index: new_index(table)?,
            rows: HashMap::new(),
            id_to_key: HashMap::new(),
            table: table.clone(),
            scope_key,
            cache_key,
            root: root.to_path_buf(),
            generation: -1,
            cache_error: None,
        };
        inner.reload_from_sqlite(true)?;
        Ok(Self {
            inner: Mutex::new(inner),
        })
    }

    /// Inserts or replaces a batch atomically in the configured partition.
    ///
    /// Duplicate chunk IDs within a batch resolve to the last value in input order.
    ///
    /// # Errors
    /// Returns a validation, SQLite, filesystem, or index error. Invalid batches leave SQLite
    /// and the in-memory index unchanged.
    pub fn upsert(&self, chunks: &[VectorChunk]) -> Result<usize, VectorStoreError> {
        if chunks.len() > MAX_BATCH_SIZE {
            return Err(VectorStoreError::BatchTooLarge {
                found: chunks.len(),
                limit: MAX_BATCH_SIZE,
            });
        }
        let mut inner = self.lock()?;
        for chunk in chunks {
            validate_vector(&inner.table, &chunk.vector)?;
        }
        if chunks.is_empty() {
            return Ok(0);
        }
        let encoded_vectors = if chunks.len() > 1 {
            encoding_pool()?.install(|| {
                chunks
                    .par_iter()
                    .map(|chunk| encode_vector(&chunk.vector))
                    .collect::<Vec<_>>()
            })
        } else {
            chunks
                .iter()
                .map(|chunk| encode_vector(&chunk.vector))
                .collect()
        };
        inner.refresh_if_stale()?;

        let table_name = inner.table.name.clone();
        let scope_key = inner.scope_key.clone();
        let table = inner.table.clone();
        let root = inner.root.clone();
        let cache_key = inner.cache_key.clone();
        let generation_before = inner.generation;
        let mut rebuilt_index = clone_index(&table, &inner.index, &root, &cache_key)?;
        let mut rows_by_key = inner.rows.clone();
        let mut id_to_key = inner.id_to_key.clone();
        let tx = inner
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let transaction_generation = partition_generation(&tx, &table_name, &scope_key)?;
        if transaction_generation != generation_before {
            let rows = read_rows(&tx, &table_name, &scope_key, table.dimension)?;
            rebuilt_index = build_index(&table, &rows)?;
            rows_by_key = rows
                .iter()
                .map(|row| (row.key, row.value.clone()))
                .collect();
            id_to_key = rows.iter().map(|row| (row.value.id, row.key)).collect();
        }
        let mut replacements = HashMap::new();
        for (chunk, vector) in chunks.iter().zip(encoded_vectors) {
            if let Some(old_key) = id_to_key.remove(&chunk.id) {
                let removed = rebuilt_index
                    .remove(old_key)
                    .map_err(|error| VectorStoreError::Index(error.to_string()))?;
                if removed != 1 {
                    return Err(VectorStoreError::Corrupt(
                        "HNSW index is missing a committed chunk".to_owned(),
                    ));
                }
                rows_by_key.remove(&old_key);
            }
            tx.execute(
                "DELETE FROM vector_chunks WHERE table_name = ?1 AND scope = ?2 AND id = ?3",
                params![
                    table_name,
                    scope_key.as_slice(),
                    chunk.id.as_bytes().as_slice()
                ],
            )?;
            tx.execute(
                "INSERT INTO vector_chunks(table_name, scope, id, payload, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    table_name,
                    scope_key.as_slice(),
                    chunk.id.as_bytes().as_slice(),
                    chunk.payload.as_bytes().as_slice(),
                    vector
                ],
            )?;
            let key = u64::try_from(tx.last_insert_rowid()).map_err(|_| {
                VectorStoreError::Corrupt("SQLite generated a non-positive row id".to_owned())
            })?;
            let value = StoredRow {
                id: chunk.id,
                payload: chunk.payload,
            };
            replacements.insert(chunk.id, (key, value, chunk.vector.clone()));
        }
        let generation = increment_generation(&tx, &table_name, &scope_key)?;
        let replacements = replacements.into_values().collect::<Vec<_>>();
        rebuilt_index
            .reserve(rebuilt_index.size().saturating_add(replacements.len()))
            .map_err(|error| VectorStoreError::Index(error.to_string()))?;
        for (key, value, vector) in replacements {
            rebuilt_index
                .add(key, &vector)
                .map_err(|error| VectorStoreError::Index(error.to_string()))?;
            id_to_key.insert(value.id, key);
            rows_by_key.insert(key, value);
        }
        let staged = stage_index(&root, &cache_key, generation, &rebuilt_index)?;
        if let Err(error) = tx.commit() {
            remove_if_exists(&staged.temporary);
            return Err(VectorStoreError::Sql(error));
        }

        inner.index = rebuilt_index;
        inner.rows = rows_by_key;
        inner.id_to_key = id_to_key;
        inner.generation = generation;
        inner.publish_cache(&table_name, &scope_key, generation, &staged);
        Ok(chunks.len())
    }

    /// Searches this table and scope, optionally restricting results by payload reference.
    ///
    /// Payload filters are evaluated across the full scoped index before the result limit, so
    /// ineligible near neighbors cannot hide eligible results.
    ///
    /// # Errors
    /// Returns a query validation, SQLite, or index error.
    pub fn search_filtered(
        &self,
        query: &[f32],
        k: usize,
        filter: &PayloadFilter,
    ) -> Result<Vec<VectorHit>, VectorStoreError> {
        let mut inner = self.lock()?;
        validate_vector(&inner.table, query)?;
        inner.refresh_if_stale()?;
        if k == 0 || inner.index.size() == 0 || filter.allowed_is_empty() {
            return Ok(Vec::new());
        }
        let matches = inner
            .index
            .exact_search(query, inner.index.size())
            .map_err(|error| VectorStoreError::Index(error.to_string()))?;
        hits_from_matches(&inner, matches.keys, matches.distances, Some(filter), k)
    }

    /// Searches this table and scope using HNSW nearest-neighbor search.
    ///
    /// # Errors
    /// Returns a query validation, SQLite, or index error.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>, VectorStoreError> {
        let mut inner = self.lock()?;
        validate_vector(&inner.table, query)?;
        inner.refresh_if_stale()?;
        if k == 0 || inner.index.size() == 0 {
            return Ok(Vec::new());
        }
        let matches = inner
            .index
            .search(query, k.min(inner.index.size()))
            .map_err(|error| VectorStoreError::Index(error.to_string()))?;
        hits_from_matches(&inner, matches.keys, matches.distances, None, k)
    }

    /// Deletes matching chunk IDs in this handle's fixed table and scope.
    ///
    /// # Errors
    /// Returns a SQLite or index error.
    pub fn delete(&self, ids: &[Cid]) -> Result<usize, VectorStoreError> {
        let mut inner = self.lock()?;
        if ids.is_empty() {
            return Ok(0);
        }
        let table_name = inner.table.name.clone();
        let scope_key = inner.scope_key.clone();
        let table = inner.table.clone();
        let root = inner.root.clone();
        let cache_key = inner.cache_key.clone();
        inner.refresh_if_stale()?;
        let generation_before = inner.generation;
        let mut rebuilt_index = clone_index(&table, &inner.index, &root, &cache_key)?;
        let mut rows_by_key = inner.rows.clone();
        let mut id_to_key = inner.id_to_key.clone();
        let tx = inner
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let transaction_generation = partition_generation(&tx, &table_name, &scope_key)?;
        if transaction_generation != generation_before {
            let current_rows = read_rows(&tx, &table_name, &scope_key, table.dimension)?;
            rebuilt_index = build_index(&table, &current_rows)?;
            rows_by_key = current_rows
                .iter()
                .map(|row| (row.key, row.value.clone()))
                .collect();
            id_to_key = current_rows
                .iter()
                .map(|row| (row.value.id, row.key))
                .collect();
        }
        let mut deleted = 0_usize;
        for id in ids {
            let key = id_to_key.remove(id);
            let count = tx.execute(
                "DELETE FROM vector_chunks WHERE table_name = ?1 AND scope = ?2 AND id = ?3",
                params![table_name, scope_key.as_slice(), id.as_bytes().as_slice()],
            )?;
            deleted = deleted.saturating_add(count);
            if count > 0
                && let Some(key) = key
            {
                let removed = rebuilt_index
                    .remove(key)
                    .map_err(|error| VectorStoreError::Index(error.to_string()))?;
                if removed != 1 {
                    return Err(VectorStoreError::Corrupt(
                        "HNSW index is missing a committed chunk".to_owned(),
                    ));
                }
                rows_by_key.remove(&key);
            }
        }
        if deleted == 0 {
            tx.commit()?;
            return Ok(0);
        }
        let generation = increment_generation(&tx, &table_name, &scope_key)?;
        let staged = stage_index(&root, &cache_key, generation, &rebuilt_index)?;
        if let Err(error) = tx.commit() {
            remove_if_exists(&staged.temporary);
            return Err(VectorStoreError::Sql(error));
        }

        inner.index = rebuilt_index;
        inner.rows = rows_by_key;
        inner.id_to_key = id_to_key;
        inner.generation = generation;
        inner.publish_cache(&table_name, &scope_key, generation, &staged);
        Ok(deleted)
    }

    /// Last durable-index publication error, if SQLite committed rows while the optional HNSW
    /// cache could not be refreshed. SQLite remains authoritative and reopening rebuilds it.
    pub fn index_cache_error(&self) -> Result<Option<String>, VectorStoreError> {
        let inner = self.lock()?;
        Ok(inner.cache_error.clone())
    }

    /// Number of vectors in this table and scope.
    ///
    /// # Errors
    /// Returns a SQLite error.
    pub fn count(&self) -> Result<u64, VectorStoreError> {
        let mut inner = self.lock()?;
        inner.refresh_if_stale()?;
        u64::try_from(inner.rows.len())
            .map_err(|_| VectorStoreError::Corrupt("row count does not fit u64".to_owned()))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StoreInner>, VectorStoreError> {
        self.inner
            .lock()
            .map_err(|_| VectorStoreError::LockPoisoned)
    }
}

impl VectorStore for HnswStore {
    fn upsert(&self, chunks: &[VectorChunk]) -> Result<usize, VectorError> {
        HnswStore::upsert(self, chunks).map_err(to_sdk_error)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<VectorHit>, VectorError> {
        HnswStore::search(self, query, k).map_err(to_sdk_error)
    }

    fn delete(&self, ids: &[Cid]) -> Result<usize, VectorError> {
        HnswStore::delete(self, ids).map_err(to_sdk_error)
    }

    fn count(&self) -> Result<u64, VectorError> {
        HnswStore::count(self).map_err(to_sdk_error)
    }
}

impl StoreInner {
    fn publish_cache(
        &mut self,
        table_name: &str,
        scope: &[u8],
        generation: i64,
        staged: &StagedIndex,
    ) {
        match publish_index(
            &self.connection,
            &self.root,
            table_name,
            scope,
            generation,
            staged,
        ) {
            Ok(cleanup_error) => self.cache_error = cleanup_error,
            Err(error) => {
                remove_if_exists(&staged.temporary);
                self.cache_error = Some(error.to_string());
            }
        }
    }

    fn refresh_if_stale(&mut self) -> Result<(), VectorStoreError> {
        let generation = partition_generation(&self.connection, &self.table.name, &self.scope_key)?;
        if generation != self.generation {
            self.reload_from_sqlite(false)?;
        }
        Ok(())
    }

    fn reload_from_sqlite(&mut self, opening: bool) -> Result<(), VectorStoreError> {
        let (generation, indexed_generation, index_file) =
            partition_state(&self.connection, &self.table.name, &self.scope_key)?;
        let rows = read_rows(
            &self.connection,
            &self.table.name,
            &self.scope_key,
            self.table.dimension,
        )?;
        if generation == indexed_generation
            && let Some(name) = index_file.as_deref()
            && let Some(path) = safe_index_path(&self.root, name)
            && path.is_file()
        {
            let index = new_index(&self.table)?;
            if index.load(&path.to_string_lossy()).is_ok() && index.size() == rows.len() {
                self.index = index;
                self.rows = rows
                    .iter()
                    .map(|row| (row.key, row.value.clone()))
                    .collect();
                self.id_to_key = rows.iter().map(|row| (row.value.id, row.key)).collect();
                self.generation = generation;
                return Ok(());
            }
        }

        let index = build_index(&self.table, &rows)?;
        let staged = stage_index(&self.root, &self.cache_key, generation, &index)?;
        self.index = index;
        self.rows = rows
            .iter()
            .map(|row| (row.key, row.value.clone()))
            .collect();
        self.id_to_key = rows.iter().map(|row| (row.value.id, row.key)).collect();
        self.generation = generation;
        match publish_index(
            &self.connection,
            &self.root,
            &self.table.name,
            &self.scope_key,
            generation,
            &staged,
        ) {
            Ok(cleanup_error) => self.cache_error = cleanup_error,
            Err(error) => {
                remove_if_exists(&staged.temporary);
                remove_if_exists(&staged.final_path);
                if opening {
                    return Err(error);
                }
                self.cache_error = Some(error.to_string());
            }
        }
        Ok(())
    }
}

struct IndexedRow {
    key: u64,
    value: StoredRow,
    vector: Vec<f32>,
}

fn validate_vector(table: &TableSpec, vector: &[f32]) -> Result<(), VectorStoreError> {
    if vector.len() != table.dimension {
        return Err(VectorStoreError::DimensionMismatch {
            expected: table.dimension,
            found: vector.len(),
        });
    }
    if vector
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
    {
        return Err(VectorStoreError::InvalidVector(
            "coordinates must be finite and within +/-1,000,000".to_owned(),
        ));
    }
    if table.metric == Metric::Cosine && vector.iter().all(|value| *value == 0.0) {
        return Err(VectorStoreError::InvalidVector(
            "cosine vectors must have non-zero magnitude".to_owned(),
        ));
    }
    Ok(())
}

fn new_index(table: &TableSpec) -> Result<Index, VectorStoreError> {
    let metric = match table.metric {
        Metric::Cosine => MetricKind::Cos,
        Metric::L2 => MetricKind::L2sq,
        Metric::Dot => MetricKind::IP,
    };
    let options = IndexOptions {
        dimensions: table.dimension,
        metric,
        quantization: ScalarKind::F32,
        connectivity: 16,
        expansion_add: 128,
        expansion_search: 64,
        ..Default::default()
    };
    Index::new(&options).map_err(|error| VectorStoreError::Index(error.to_string()))
}

fn encoding_pool() -> Result<&'static rayon::ThreadPool, VectorStoreError> {
    if ENCODING_POOL.get().is_none() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(MAX_ENCODING_THREADS)
            .build()
            .map_err(|error| VectorStoreError::WorkerPool(error.to_string()))?;
        let _ = ENCODING_POOL.set(pool);
    }
    ENCODING_POOL
        .get()
        .ok_or_else(|| VectorStoreError::WorkerPool("pool initialization failed".to_owned()))
}

fn build_index(table: &TableSpec, rows: &[IndexedRow]) -> Result<Index, VectorStoreError> {
    let index = new_index(table)?;
    index
        .reserve(rows.len())
        .map_err(|error| VectorStoreError::Index(error.to_string()))?;
    for row in rows {
        index
            .add(row.key, &row.vector)
            .map_err(|error| VectorStoreError::Index(error.to_string()))?;
    }
    Ok(index)
}

fn clone_index(
    table: &TableSpec,
    source: &Index,
    root: &Path,
    key: &str,
) -> Result<Index, VectorStoreError> {
    let nonce = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = root.join(format!("{key}-clone-{}-{nonce}.tmp", std::process::id()));
    source
        .save(&temporary.to_string_lossy())
        .map_err(|error| VectorStoreError::Index(error.to_string()))?;
    let cloned = new_index(table)?;
    let load_result = cloned.load(&temporary.to_string_lossy());
    remove_if_exists(&temporary);
    load_result.map_err(|error| VectorStoreError::Index(error.to_string()))?;
    Ok(cloned)
}

fn read_rows(
    connection: &Connection,
    table_name: &str,
    scope: &[u8],
    dimension: usize,
) -> Result<Vec<IndexedRow>, VectorStoreError> {
    let mut statement = connection.prepare(
        "SELECT row_id, id, payload, vector FROM vector_chunks
         WHERE table_name = ?1 AND scope = ?2 ORDER BY row_id",
    )?;
    let raw_rows = statement.query_map(params![table_name, scope], |row| {
        Ok(RawRow {
            row_id: row.get(0)?,
            id: row.get(1)?,
            payload: row.get(2)?,
            vector: row.get(3)?,
        })
    })?;
    let mut rows = Vec::new();
    for raw in raw_rows {
        let raw = raw?;
        let key = u64::try_from(raw.row_id)
            .map_err(|_| VectorStoreError::Corrupt("non-positive SQLite row id".to_owned()))?;
        let id_bytes: [u8; 32] = raw.id.try_into().map_err(|value: Vec<u8>| {
            VectorStoreError::Corrupt(format!("chunk id has {} bytes, expected 32", value.len()))
        })?;
        let payload_bytes: [u8; 32] = raw.payload.try_into().map_err(|value: Vec<u8>| {
            VectorStoreError::Corrupt(format!("payload id has {} bytes, expected 32", value.len()))
        })?;
        let (coordinates, remainder) = raw.vector.as_slice().as_chunks::<4>();
        if !remainder.is_empty() {
            return Err(VectorStoreError::Corrupt(
                "vector byte length is not a multiple of four".to_owned(),
            ));
        }
        let mut vector = Vec::with_capacity(coordinates.len());
        for bytes in coordinates {
            vector.push(f32::from_le_bytes(*bytes));
        }
        if vector.len() != dimension {
            return Err(VectorStoreError::Corrupt(format!(
                "stored vector has {} coordinates, table declares {dimension}",
                vector.len()
            )));
        }
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(VectorStoreError::Corrupt(
                "stored vector contains a non-finite coordinate".to_owned(),
            ));
        }
        rows.push(IndexedRow {
            key,
            value: StoredRow {
                id: Cid::from_bytes(id_bytes),
                payload: Cid::from_bytes(payload_bytes),
            },
            vector,
        });
    }
    Ok(rows)
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len().saturating_mul(4));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn increment_generation(
    tx: &Transaction<'_>,
    table_name: &str,
    scope: &[u8],
) -> Result<i64, VectorStoreError> {
    let changed = tx.execute(
        "UPDATE vector_partitions SET generation = generation + 1
         WHERE table_name = ?1 AND scope = ?2 AND generation < 9223372036854775807",
        params![table_name, scope],
    )?;
    if changed != 1 {
        return Err(VectorStoreError::Corrupt(
            "vector partition missing or generation exhausted".to_owned(),
        ));
    }
    partition_generation(tx, table_name, scope)
}

fn partition_generation(
    connection: &Connection,
    table_name: &str,
    scope: &[u8],
) -> Result<i64, VectorStoreError> {
    connection
        .query_row(
            "SELECT generation FROM vector_partitions WHERE table_name = ?1 AND scope = ?2",
            params![table_name, scope],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| VectorStoreError::Corrupt("vector partition is missing".to_owned()))
}

fn partition_state(
    connection: &Connection,
    table_name: &str,
    scope: &[u8],
) -> Result<(i64, i64, Option<String>), VectorStoreError> {
    connection
        .query_row(
            "SELECT generation, indexed_generation, index_file FROM vector_partitions
             WHERE table_name = ?1 AND scope = ?2",
            params![table_name, scope],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| VectorStoreError::Corrupt("vector partition is missing".to_owned()))
}

fn cache_key(table_name: &str, scope: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(table_name.as_bytes());
    hasher.update(&[0]);
    hasher.update(scope);
    hasher.finalize().to_hex().to_string()
}

struct StagedIndex {
    temporary: PathBuf,
    final_path: PathBuf,
    file_name: String,
}

fn stage_index(
    root: &Path,
    key: &str,
    generation: i64,
    index: &Index,
) -> Result<StagedIndex, VectorStoreError> {
    let nonce = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let stem = format!("{key}-{generation}-{}-{nonce}", std::process::id());
    let temporary = root.join(format!("{stem}.tmp"));
    let final_path = root.join(format!("{stem}.usearch"));
    index
        .save(&temporary.to_string_lossy())
        .map_err(|error| VectorStoreError::Index(error.to_string()))?;
    OpenOptions::new()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    let file_name = final_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| VectorStoreError::Corrupt("index cache filename is invalid".to_owned()))?
        .to_owned();
    Ok(StagedIndex {
        temporary,
        final_path,
        file_name,
    })
}

fn publish_index(
    connection: &Connection,
    root: &Path,
    table_name: &str,
    scope: &[u8],
    generation: i64,
    staged: &StagedIndex,
) -> Result<Option<String>, VectorStoreError> {
    if let Err(error) = fs::rename(&staged.temporary, &staged.final_path) {
        remove_if_exists(&staged.temporary);
        return Err(VectorStoreError::Io(error));
    }
    let previous = match connection
        .query_row(
            "SELECT index_file FROM vector_partitions
             WHERE table_name = ?1 AND scope = ?2 AND generation = ?3",
            params![table_name, scope, generation],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
    {
        Ok(previous) => previous.flatten(),
        Err(error) => {
            remove_if_exists(&staged.final_path);
            return Err(VectorStoreError::Sql(error));
        }
    };
    let changed = match connection.execute(
        "UPDATE vector_partitions SET indexed_generation = ?3, index_file = ?4
         WHERE table_name = ?1 AND scope = ?2 AND generation = ?3",
        params![table_name, scope, generation, staged.file_name],
    ) {
        Ok(changed) => changed,
        Err(error) => {
            remove_if_exists(&staged.final_path);
            return Err(VectorStoreError::Sql(error));
        }
    };
    if changed == 0 {
        remove_if_exists(&staged.final_path);
        return Ok(None);
    }
    if let Some(previous) = previous
        && previous != staged.file_name
    {
        let Some(previous_path) = safe_index_path(root, &previous) else {
            return Ok(Some(
                "obsolete index cache path was invalid and could not be reclaimed".to_owned(),
            ));
        };
        match fs::remove_file(previous_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Ok(Some(format!(
                    "could not reclaim obsolete index cache: {error}"
                )));
            }
        }
    }
    Ok(None)
}

fn safe_index_path(root: &Path, file_name: &str) -> Option<PathBuf> {
    let path = Path::new(file_name);
    if path.file_name() != Some(OsStr::new(file_name))
        || path.extension() != Some(OsStr::new("usearch"))
    {
        return None;
    }
    Some(root.join(path))
}

fn remove_if_exists(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

fn hits_from_matches(
    inner: &StoreInner,
    keys: Vec<u64>,
    distances: Vec<f32>,
    filter: Option<&PayloadFilter>,
    limit: usize,
) -> Result<Vec<VectorHit>, VectorStoreError> {
    let mut hits = Vec::new();
    for (key, distance) in keys.into_iter().zip(distances) {
        let Some(row) = inner.rows.get(&key) else {
            continue;
        };
        if filter.is_some_and(|payload_filter| !payload_filter.allows(&row.payload)) {
            continue;
        }
        let score = match inner.table.metric {
            Metric::Cosine => 1.0 - distance,
            Metric::L2 => -distance,
            Metric::Dot => 1.0 - distance,
        };
        if !score.is_finite() {
            return Err(VectorStoreError::Corrupt(
                "usearch returned a non-finite score".to_owned(),
            ));
        }
        hits.push(VectorHit {
            id: row.id,
            score,
            payload: row.payload,
        });
        if hits.len() >= limit {
            break;
        }
    }
    Ok(hits)
}

fn to_sdk_error(error: VectorStoreError) -> VectorError {
    match error {
        VectorStoreError::DimensionMismatch { expected, found } => {
            VectorError::DimensionMismatch { expected, found }
        }
        VectorStoreError::Corrupt(message) => VectorError::Corrupt(message),
        other => VectorError::Io(other.to_string()),
    }
}
