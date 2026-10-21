//! The journal: gap-free sequence numbers assigned inside the write transaction, read
//! transactions that never block the writer, checkpoints referencing CAS snapshots.

use std::path::PathBuf;

use polaris_sdk::contracts::EventRecord;
use polaris_sdk::ids::{Cid, Seq};
use polaris_sdk::registry::{EventStore, EventStoreError};
use redb::{Database, DatabaseError, ReadableDatabase, ReadableTable, TableDefinition};

use crate::codec::{decode_record, encode_record};
use crate::error::EventLogError;

/// Name of the journal table (`u64` sequence number → encoded record).
pub const EVENTS_TABLE: &str = "events";
pub(crate) const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new(EVENTS_TABLE);
pub(crate) const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
pub(crate) const CHECKPOINTS: TableDefinition<u64, &[u8]> = TableDefinition::new("checkpoints");
pub(crate) const LOCKS: TableDefinition<&str, &[u8]> = TableDefinition::new("locks");
pub(crate) const L2_CACHE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("l2_cache");

/// Version of the table layout in this file.
const LAYOUT_VERSION: u64 = 1;

/// When a commit is durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Durability {
    /// Durable when `append` returns (fsync).
    #[default]
    Immediate,
    /// Faster; the last commits may be lost on power loss (redb `Durability::None`).
    Eventual,
}

/// Event-log configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLogConfig {
    /// Database file (`~/.polaris/events.redb` or `.polaris/events.redb`).
    pub path: PathBuf,
    /// Commit durability for events.
    pub durability: Durability,
    /// Process that owns leases taken through this handle (stale-lock detection).
    pub owner_pid: u32,
    /// Largest L2 exact-cache value in bytes.
    pub max_cache_value_bytes: usize,
}

impl EventLogConfig {
    /// Immediate durability, the current process as lease owner, 8 MiB cache values.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            durability: Durability::Immediate,
            owner_pid: std::process::id(),
            max_cache_value_bytes: 8 << 20,
        }
    }
}

/// A checkpoint: the session state at `at` is the CAS snapshot `snapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    /// Last event covered by the snapshot.
    pub at: Seq,
    /// Snapshot in CAS.
    pub snapshot: Cid,
}

/// Append-only event journal on redb (one owner per file).
#[derive(Debug)]
pub struct EventLog {
    pub(crate) db: Database,
    durability: redb::Durability,
    pub(crate) owner_pid: u32,
    pub(crate) max_cache_value_bytes: usize,
}

impl EventLog {
    /// Opens or creates the log.
    ///
    /// # Errors
    /// [`EventLogError::Locked`] when another handle owns the file, [`EventLogError::Open`] when it
    /// cannot be opened, [`EventLogError::SchemaTooNew`] for a newer layout.
    pub fn open(cfg: EventLogConfig) -> Result<Self, EventLogError> {
        let db = Database::create(&cfg.path).map_err(|e| match e {
            DatabaseError::DatabaseAlreadyOpen => EventLogError::Locked,
            other => EventLogError::Open(other.to_string()),
        })?;
        let txn = db.begin_write()?;
        {
            txn.open_table(EVENTS)?;
            txn.open_table(CHECKPOINTS)?;
            txn.open_table(LOCKS)?;
            txn.open_table(L2_CACHE)?;
            let mut meta = txn.open_table(META)?;
            let layout = meta.get("layout")?.map(|g| g.value());
            match layout {
                None => {
                    meta.insert("layout", LAYOUT_VERSION)?;
                }
                Some(found) if found > LAYOUT_VERSION => {
                    return Err(EventLogError::SchemaTooNew {
                        found: u16::try_from(found).unwrap_or(u16::MAX),
                        supported: u16::try_from(LAYOUT_VERSION).unwrap_or(u16::MAX),
                    });
                }
                Some(_) => {}
            }
        }
        txn.commit()?;
        let durability = match cfg.durability {
            Durability::Immediate => redb::Durability::Immediate,
            Durability::Eventual => redb::Durability::None,
        };
        Ok(Self {
            db,
            durability,
            owner_pid: cfg.owner_pid,
            max_cache_value_bytes: cfg.max_cache_value_bytes,
        })
    }

    /// Appends one record (its own `seq` is ignored) and returns the assigned sequence number.
    ///
    /// # Errors
    /// [`EventLogError::Db`] / [`EventLogError::Codec`].
    pub fn append(&self, record: &EventRecord) -> Result<Seq, EventLogError> {
        self.append_batch(std::slice::from_ref(record))
    }

    /// Appends records in one transaction (all or none) and returns the last sequence number; an
    /// empty batch returns the head unchanged.
    ///
    /// # Errors
    /// [`EventLogError::Db`] / [`EventLogError::Codec`].
    pub fn append_batch(&self, records: &[EventRecord]) -> Result<Seq, EventLogError> {
        if records.is_empty() {
            return self.head();
        }
        let mut txn = self.db.begin_write()?;
        txn.set_durability(self.durability)?;
        let last = {
            let mut table = txn.open_table(EVENTS)?;
            let mut seq = table.last()?.map_or(0, |(k, _)| k.value());
            for record in records {
                seq = seq
                    .checked_add(1)
                    .ok_or_else(|| EventLogError::Codec("sequence number overflow".to_owned()))?;
                let bytes = encode_record(&record.with_seq(Seq::new(seq)))?;
                table.insert(seq, bytes.as_slice())?;
            }
            seq
        };
        txn.commit()?;
        Ok(Seq::new(last))
    }

    /// Records with `from <= seq <= to`, in order.
    ///
    /// # Errors
    /// [`EventLogError::Db`], [`EventLogError::Codec`] / [`EventLogError::SchemaTooNew`] for a bad
    /// record.
    pub fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventLogError> {
        if from > to {
            return Ok(Vec::new());
        }
        let txn = self.db.begin_read()?;
        let table = txn.open_table(EVENTS)?;
        let mut out = Vec::new();
        for item in table.range(from.get()..=to.get())? {
            let (_, value) = item?;
            out.push(decode_record(value.value())?);
        }
        Ok(out)
    }

    /// Events `1..=until` (`/session replay [step]`).
    ///
    /// # Errors
    /// As [`EventLog::read_range`].
    pub fn replay(&self, until: Seq) -> Result<Vec<EventRecord>, EventLogError> {
        self.read_range(Seq::new(1), until)
    }

    /// The last sequence number (`Seq::ZERO` for an empty log).
    ///
    /// # Errors
    /// [`EventLogError::Db`].
    pub fn head(&self) -> Result<Seq, EventLogError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(EVENTS)?;
        Ok(Seq::new(table.last()?.map_or(0, |(k, _)| k.value())))
    }

    /// Records that the state at `at` is the CAS snapshot `snapshot`.
    ///
    /// # Errors
    /// [`EventLogError::CheckpointBeyondHead`], [`EventLogError::Db`].
    pub fn checkpoint(&self, at: Seq, snapshot: Cid) -> Result<(), EventLogError> {
        let head = self.head()?;
        if at > head {
            return Err(EventLogError::CheckpointBeyondHead { at, head });
        }
        let txn = self.db.begin_write()?;
        {
            let mut table = txn.open_table(CHECKPOINTS)?;
            table.insert(at.get(), snapshot.as_bytes().as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// The newest checkpoint at or before `at_or_before`.
    ///
    /// # Errors
    /// [`EventLogError::Db`], [`EventLogError::Codec`] for a malformed checkpoint row.
    pub fn latest_checkpoint(
        &self,
        at_or_before: Seq,
    ) -> Result<Option<Checkpoint>, EventLogError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CHECKPOINTS)?;
        let Some(item) = table.range(..=at_or_before.get())?.next_back() else {
            return Ok(None);
        };
        let (at, value) = item?;
        let bytes = <[u8; 32]>::try_from(value.value())
            .map_err(|_| EventLogError::Codec("checkpoint snapshot is not 32 bytes".to_owned()))?;
        Ok(Some(Checkpoint {
            at: Seq::new(at.value()),
            snapshot: Cid::from_bytes(bytes),
        }))
    }
}

impl EventStore for EventLog {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        Ok(Self::append(self, record)?)
    }

    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(Self::read_range(self, from, to)?)
    }
}
