# Polaris vendor backport

This directory contains the exact `docx-rs` 0.4.22 published crate source, with
the archive SHA-256 and per-file hashes recorded in `UPSTREAM-CHECKSUMS.json`.
The published source revision is `f04cf8b47a925b2b6fff850dcb75069aae89dab4`.
The crate archive omits its MIT license file, so `LICENSE` is copied byte-for-byte
from that exact upstream revision (SHA-256
`718ca36cb18f8ee3ddf9fdffc5f12fc17fad6328d40d8f13bee0a6daa7e1416e`).

Polaris applies only the upstream ZIP 9 migration from commit
`ad7fede6a7f3b61a097c28d0c39dd04626e44701`: both Cargo manifests require ZIP
9.0.4, and the existing test fixture's `FileOptions` type uses ZIP 9's second
lifetime. The upstream `entry.name()?` change targets newer monorepo reader
code that is absent from the published crate archive; the corresponding
published source file remains byte-for-byte pinned. The production workspace
lock resolves the semver-compatible ZIP dependency to 9.1.1.
The [upstream ZIP 9 security migration commit](https://github.com/bokuweb/docx-rs/commit/ad7fede6a7f3b61a097c28d0c39dd04626e44701)
is the source for this backport.

The parser disables docx-rs default features and explicitly disables image
previews. Images are not decoded as previews during document parsing.

Run `python -I @prime/tools/check-vendored-docx.py` from the repository root to
verify exact source provenance, the narrow patch set, and the absence of added
or missing files. The checker itself pins the checksum manifest digest.
