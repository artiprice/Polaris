//! Tool facade. Children never depend on this facade.
//!
//! P1 exposes E0 workspace inspection and pure queries. P2 owns proposal-producing writes and
//! sandboxed execution; P5 owns network tools. The P3 math child provides a pure typed dispatcher.
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

/// Render the registered P1 and P2 builtin schemas as documentation. Regenerate with the `catalog` example.
#[must_use]
pub fn tool_catalog() -> String {
    let mut out = String::from(
        "# Registered tools\n\nGenerated from P1 and P2 builtin schemas plus typed tool specs; checked by the facade's catalog test.\n\n| Name | Effects | Tier | Phase | Description |\n|---|---|---|---|---|\n",
    );
    let mut builtins = builtin::builtin_specs();
    builtins.extend(builtin::p2_builtin_specs());
    builtins.sort_by(|left, right| left.name.cmp(&right.name));
    for spec in builtins {
        let effects = if spec.effects.reads_workspace {
            if spec.effects.writes_workspace {
                "workspace proposal"
            } else {
                "workspace read"
            }
        } else if spec.effects.executes {
            "sandbox execution"
        } else {
            "pure / persisted context read"
        };
        let phase = if spec.effects.writes_workspace
            || spec.effects.executes
            || spec.name.as_ref() == "classify_command"
        {
            "P2"
        } else {
            "P1"
        };
        out.push_str(&format!(
            "| `{}` | {} | E0 | {} | {} |\n",
            spec.name, effects, phase, spec.description
        ));
    }
    let math = math::MathTool.tool_spec();
    out.push_str(&format!(
        "| `{}` | pure | E0 | P3 | {} |\n",
        math.name, math.description
    ));
    out.push_str("\nNetwork tools and remote MCP integration are future-phase work. The math dispatcher is a pure P3 tool.\n");
    out
}
