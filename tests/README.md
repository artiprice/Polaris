# tests — every test in the workspace

All tests live here, in a tree that mirrors the paths they test. Package sources contain no
test code.

| Path | What it holds | How it is compiled |
|---|---|---|
| `tests/<package path>/<name>.rs` | Integration tests of that package, e.g. `tests/crates/polaris-kernel/agent_tools.rs` for `crates/polaris-kernel` | A `[[test]]` target in the package's `Cargo.toml` (`autotests = false`) |
| `tests/<package path>/src/<file>.rs` | Unit tests of exactly that source file, e.g. `tests/crates/polaris-kernel/src/frame_mutation.rs` for `crates/polaris-kernel/src/frame_mutation.rs` | `#[cfg(test)] #[path = "…"] mod tests;` in the source file, so the tests keep private access |
| `tests/<package path>/<dir>/` | Helper modules, snapshots, proptest regressions and data that belonged to those tests | Referenced with paths relative to the test file |
| `tests/tests/`, `tests/src/` | Cross-crate suites and helpers (`polaris-tests` package) | Integration tests of `polaris-tests` |
| `tests/fixtures/` | Shared test data (see its README) | Read at test time |
| `tests/spike/`, `tests/fuzz/` | Spike packages and fuzz targets | Their own packages |

Adding a test:

- Unit test for `crates/foo/src/bar.rs`: put it in `tests/crates/foo/src/bar.rs` and declare it in
  `bar.rs` with `#[cfg(test)] #[path = "<relative path>"] mod tests;`.
- Integration test for package `crates/foo`: put it in `tests/crates/foo/<name>.rs` and add
  `[[test]] name = "<name>" path = "../../tests/crates/foo/<name>.rs"` to `crates/foo/Cargo.toml`.

Run everything with `cargo test --workspace`; one package with `cargo test -p <package>`.
Production code denies `unsafe` and panicking shortcuts (`[workspace.lints]`); tests may
`unwrap`/`expect`/`panic` (`clippy.toml`).
