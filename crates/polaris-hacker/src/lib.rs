//! Deterministic zero-model-token source auditing and bounded sandbox stress execution.
//!
//! Static heuristics are advisory and always carry source spans and rule evidence. Dynamic
//! command outcomes are observations; an exit status does not by itself prove a defect or a
//! sanitizer finding. Sandbox isolation is supplied by the caller's SDK backend and policy.

mod boundary;
mod dynamic;
mod error;
mod pipeline;
mod report;
mod static_scan;

pub use boundary::{BoundaryGen, BoundaryLimits};
pub use dynamic::{DynamicReport, DynamicRun, Sanitizer, StressPlan, run_dynamic_stress};
pub use error::HackerError;
pub use pipeline::audit;
pub use report::{AuditReport, Confidence, Finding, FindingClass, SourceFile, StaticReport};
pub use static_scan::static_scan;
