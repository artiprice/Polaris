//! Tool facade. Children never depend on this facade.
//!
//! P1 exposes E0 workspace inspection and pure queries. P2 owns proposal-producing writes and
//! sandboxed execution; P5 owns network tools. The math child is tracked with its P3 phase.
//!
//! ```
//! use polaris_tools::{builtin, rtk};
//! use polaris_tools::math as _;
//! assert!(!builtin::builtin_specs().is_empty());
//! let _ = core::mem::size_of::<rtk::RtkError>();
//! ```
#![doc = include_str!("../../../docs/tool-catalog.md")]

pub use polaris_tools_builtin as builtin;
pub use polaris_tools_math as math;
pub use polaris_tools_rtk as rtk;

/// Render the registered P1 schemas as documentation. Regenerate with the `catalog` example.
#[must_use]
pub fn tool_catalog() -> String {
    let mut out = String::from(
        "# Registered P1 tools\n\nGenerated from `builtin_specs()`; checked by the facade's catalog test.\n\n| Name | Effects | Tier | Phase | Description |\n|---|---|---|---|---|\n",
    );
    for spec in builtin::builtin_specs() {
        let effects = if spec.effects.reads_workspace {
            "workspace read"
        } else {
            "pure / persisted context read"
        };
        out.push_str(&format!(
            "| `{}` | {} | E0 | P1 | {} |\n",
            spec.name, effects, spec.description
        ));
    }
    out.push_str("\nProposal-producing writes and sandboxed process execution are P2. Network tools are P5. These later tools are absent from the P1 registry.\n");
    out
}
