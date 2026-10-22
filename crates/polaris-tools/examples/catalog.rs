//! Emit the real tool catalog for docs/tool-catalog.md.
use std::io::Write;
fn main() -> std::io::Result<()> {
    std::io::stdout()
        .lock()
        .write_all(polaris_tools::tool_catalog().as_bytes())
}
