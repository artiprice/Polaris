//! # polaris-storage-eventlog
//!
//! Append-only event journal on redb (`events.redb`), implementing the sdk `EventStore` (the bus
//! publishes into it) and `ExactCacheStore` (the L2 tier of `polaris-storage-cache`).
//!
//! - Sequence numbers are assigned inside the write transaction: gap-free, starting at 1; a batch
//!   is one transaction.
//! - Records are `[schema u16][JSON]` ([`codec`]); a record from a newer Polaris is reported as
//!   [`EventLogError::SchemaTooNew`], never misread.
//! - `/session replay [step]`: [`EventLog::replay`] plus [`EventLog::latest_checkpoint`] (CAS
//!   snapshots).
//! - Lease locks survive crashes; leases of another process are listed by
//!   [`EventLog::stale_locks`] for the Reaper.
//! - One owner per file: a second open (another process, or the CLI while the daemon runs) is
//!   [`EventLogError::Locked`].
//! - The API is blocking (redb I/O): async callers use a dedicated owner thread or
//!   `spawn_blocking`.

mod cache;
pub mod codec;
mod error;
mod locks;
mod log;

pub use error::EventLogError;
pub use locks::LockRecord;
pub use log::{Checkpoint, Durability, EVENTS_TABLE, EventLog, EventLogConfig};
