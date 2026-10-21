# Biscuit 6.0.0 import feature guards

This directory contains the published `biscuit-auth` 6.0.0 crate, retaining its Apache-2.0
licence and source notices. Its original 94 files are hashed in `UPSTREAM-CHECKSUMS.json`.
Published archive SHA-256: `d5884fc86b3e21f5649ef4326e17ef729b3096e6502deaf13db7b7fb05bb992b`.
Upstream revision: `0f0b4e0e6fe07220c1ba6b51bff21d450d94a975` (`biscuit-auth/`).

Five `ToAnyParam` imports in `src/token/builder/{check,fact,policy,rule,term}.rs` are moved
behind their existing `datalog-macro` feature condition. The trait and every use already have
that condition. Two further guards restrict the macro/test-only `PublicKey` builder import
and the PEM-only `parse_any_algorithm` helper to their callers' existing feature conditions.
These make warning-denied builds succeed without lint suppression. No runtime, cryptographic,
serialization or authorization logic changes. The existing workspace rustfmt check is retained:
imports are sorted and `src/format/schema.rs` receives spaces around `=` in prost attributes
and the empty `Empty` struct's braces are put on one line.
The provenance checker reverses exactly those cosmetic edits before checking published hashes;
tags, types, strings and runtime source remain byte-for-byte constrained.
The change repairs [upstream issue 305](https://github.com/eclipse-biscuit/biscuit-rust/issues/305).

Polaris uses runtime builders with default features disabled. This avoids the unused macro
dependency carrying [RUSTSEC-2026-0173](https://rustsec.org/advisories/RUSTSEC-2026-0173.html).
No advisory exception was added. `audit-as-crates-io = true` retains the dependency's cargo-vet
audit obligation; this patch is not a claim that the full library has been audited.

Verify the exact published bytes modulo those seven guards:

```powershell
python -I @prime/tools/check-vendored-biscuit.py
cargo test -p polaris-security-capability --no-fail-fast
cargo deny check
cargo vet
```

The standalone fuzz and dependency-probe workspaces reference the same source through their
workspace dependency declarations. Remove this copy only after a published upstream version
fixes the feature guards, and re-run capability, fuzz, provenance and supply-chain checks.
