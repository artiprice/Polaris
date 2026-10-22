//! # polaris-kernel-bus
//!
//! CQRS bus of the kernel. Agents and front-ends never mutate state directly: they queue
//! `Command<Unverified>`s (bounded; a full queue is a typed error that hands the command back) and
//! observe immutable `EventRecord`s that the kernel publishes.
//!
//! - **Log-then-publish:** [`Bus::publish`] appends to the injected sdk `EventStore` first and
//!   broadcasts only after the append succeeded, under one ordering lock, so subscribers never see
//!   an event that is not durable and see events in persisted order.
//! - **Lag recovery:** a slow subscriber gets one [`BusError::Lagged`] with the first missed
//!   sequence number, replays the gap from the store and continues (no frozen UI, no lost events).
//! - **Hot path (zero allocation after warm-up, PLAN ADR-003):** `try_send`, `try_recv`, fan-out
//!   and `try_next`. Events are fixed-size `Copy` records with text in CAS, so broadcast clones
//!   never allocate.

mod bus;
mod error;
#[cfg(feature = "kani")]
mod kani_proofs;
mod stream;

pub use bus::{Bus, BusConfig, BusStats, CommandReceiver};
pub use error::{BusError, SendError, TryRecvError, TrySendError};
pub use stream::EventStream;
