//! Extension adapters for skills and Model Context Protocol servers.
//!
//! Skill bodies and MCP tool descriptions are untrusted prompt input. They supply context and
//! suggestions only; authorization and tool execution remain governed by the caller's frame
//! capabilities and verified tool registry.

pub use polaris_ext_mcp as mcp;
pub use polaris_ext_skills as skills;
