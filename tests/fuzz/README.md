# tests/fuzz — cargo-fuzz targets (planned, not yet created)

This directory is **excluded** from the Cargo workspace (`exclude = ["tests/fuzz", ...]` in the root
`Cargo.toml`). It becomes a standalone cargo-fuzz project in its phase. Nothing here is built today.

## Why it is separate

- `cargo fuzz` needs a nightly toolchain and libFuzzer instrumentation; ASan/libFuzzer on
  `x86_64-pc-windows-msvc` is unreliable, so fuzzing runs **inside WSL2 (Ubuntu 24.04) on nightly**.
- An excluded manifest cannot inherit `version.workspace` / `lints.workspace`; its `Cargo.toml` uses
  literal values. `libfuzzer-sys` and `arbitrary` live only here (not in `[workspace.dependencies]`).
- The `fuzz_target!` macro expands to `unsafe` FFI glue: this is a declared, signed-off exception to the
  zero-`unsafe` rule (see `@prime/PLAN.md` → ADR "Unsafe policy").

## Planned targets (one per parser/transform; see `@prime/PLAN.md` → Tests → Fuzz)

| Target | Crate under test | Phase |
|---|---|---|
| `directives_grammar` | `polaris-kernel-directives` | P1 |
| `rtk_stream` | `polaris-tools-rtk` | P1 |
| `cas_short_ref_roundtrip` | `polaris-storage-cas` | P1 |
| `eventlog_decode` | `polaris-storage-eventlog` | P1 |
| `policy_yaml` | `polaris-openshell-policy` | P2 |
| `sync_manifest` | `polaris-storage-sync` | P2 |
| `sse_frames` | `polaris-nebius-cloud-token-factory` | P3 |
| `biscuit_token_bytes` | `polaris-security-capability` | P3 |
| `context_fold_roundtrip` | `polaris-runtime-context` | P3 |
| `pii_mask_unmask` | `polaris-security-pii` | P4 |
| `dfa_scan` | `polaris-security-automata` | P2 |
| `proto_decode` | `polaris-proto` | P1 |
| `ingest_docx` / `ingest_xlsx` / `ingest_pdf` | `polaris-ingest` | P5 |
| `diff_apply` | `polaris-tools-builtin` | P1 |

Each target asserts **no panic** plus a round-trip or invariant property (e.g. `unmask(mask(x)) == x`,
`expand(fold(x)) == x` byte-for-byte). Crashes are minimized and promoted into `tests/fixtures/fuzz/`
regression inputs (generated, never hand-edited).
