# Recorded RTK inputs

`cargo-e0308.jsonl` is a real Cargo diagnostic stream captured from a temporary, dependency-free
fixture crate with this command:

```text
cargo check --offline --quiet --message-format=json
```

`cargo-e0308.txt` is the corresponding human-readable stream from the same crate, captured with
`cargo check --offline --quiet --color never`.

The fixture intentionally keeps Cargo's diagnostic object shape and multi-span E0308 error. It
omits the non-diagnostic `build-finished` record. Recording environment:

```text
cargo 1.99.0 (5f94df478 2026-08-27)
rustc 1.99.0 (b940084d7 2026-09-28)
```

The original temporary directory was replaced with `<TEMP>` in `manifest_path` and `src_path` so
the checked-in fixture has no machine-specific path. Source was:

```rust
fn main() { let value: u32 = "wrong"; println!("{value}"); }
```
