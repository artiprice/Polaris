# Polaris — Security review (P10-2)

> **Date:** 2026-10-29 · **Branch:** `feat/chat-edits` · **Scope:** the workspace as built today on the
> reference Windows machine (Rust 1.99.0). **Method:** the threat model of PLAN §6.11 checked
> against the code and the tests that exercise each control; an inventory of `unsafe` code; a
> licence audit of the resolved dependency graph against `deny.toml`. **Not covered here:** RustSec
> advisories and `cargo vet` (the tools are not installed on this machine, see §4), an independent
> external audit, and the Nebius-hosted surfaces descoped by D10.

## 1. Threat model against the implementation

| Threat (§6.11) | Controls in the code today | Exercised by | Open |
|---|---|---|---|
| Host damage by agent code | The model cannot write files directly: every change is a proposal verified against the exact revision it was made for and written only after the person applies it (human-approved mutation evidence). Tests and CI run only in a sandbox: `/ci-run` uses a Docker container with the workspace mounted read-only, all capabilities dropped, `no-new-privileges`, CPU/memory/PID limits (E56). `/hack` and `/nv-verify` only read files. | `session/chat.rs` (review then write, decline not written), `cicd` lifecycle tests, live `/ci-run` (E56) | OpenShell/microVM isolation is not set up on this machine (P2 paused, P7-X-3); the CI container has bridge networking for cold dependency fetches |
| Credential theft | Provider keys come from an explicit dotenv file or the environment, never from a repository `.env`; configuration layers refuse `system_one.*` secrets from the repository layer; the web UI binds loopback only and checks a per-run token and the `Host` header; `polarisd` writes its client credential with `create_new` (mode 0600 on Unix) | `web_api.rs` (missing/wrong token, foreign Host → 401), config layer tests, `daemon_lifecycle.rs` | On Windows the credential file keeps the folder's default ACL; keyring storage is not wired into the frontends |
| Prompt injection (web, docs, repo) | Workspace content and `$rag` passages are labelled untrusted data and sent as user data, not instructions; destructive changes (deletes) need the person's decision; Biscuit capabilities bound every tool call to its frame and path; automata rule packs flag secrets and destructive shell commands | `session/chat.rs`, `local_search.rs`, verifier tests, `hacker` tests | The S1 safety gate runs degraded without local models |
| PII leakage to cloud | The prompt privacy layer masks PII before provider calls (PII model fp32) | kernel prompt-privacy tests | PII masking quality is bounded by the local model; no masking when it is unavailable is reported, not hidden |
| Test evasion / fake completion | Independent verifier and Proof-of-Done gate; a normal reply is stated not to be proof of completion | verifier and guards suites | — |
| Edit wars between agents | Leases on file scopes; optimistic concurrency on the base revision (`expected_base`) | leases (incl. loom) and patch tests | — |
| Supply chain | `deny.toml` (licences, advisories with `yanked = "deny"`, sources), `supply-chain/` for cargo-vet, pinned toolchain, SPDX SBOM (`cargo xtask release-sbom`) | licence audit §3 | Advisory and vet runs need the tools (§4) |
| Local RPC hijack | `polarisd` serves a `polaris-*` named pipe (private Unix socket elsewhere); every call carries a Biscuit from the daemon's own issuer, scoped to `sessions/<id>/rpc` with Read and ProposeDiff, 1 s–7 d lifetime; `SubmitText`/`ReadText` are pinned to the daemon's session and only serve text its events reference | `daemon_lifecycle.rs` (foreign issuer → UNAUTHENTICATED, other session → PERMISSION_DENIED, read-only cannot write, unreferenced digest → NOT_FOUND), attach tests (E53) | mTLS remote deployment is implemented in `polaris-rpc` but not deployed |
| Secrets in RAM / logs | `secrecy`/`zeroize` for keys; the automata redactor runs before log sinks and on CI logs | redactor tests, CI engine log path | — |

## 2. `unsafe` code

- The workspace sets `unsafe_code = "deny"` in `[workspace.lints.rust]`, and 15 crate roots also
  `#![forbid(unsafe_code)]`.
- First-party source has **no `unsafe` blocks, functions or impls** (search of `crates/`, `apps/`,
  `xtask/` outside tests and Kani harnesses). The single `allow(unsafe_code)` note is in
  `polaris-ui-gui`, which keeps `deny` rather than `forbid` because the Slint-generated module marks
  its own items; that code is produced by Slint, not written here.
- Unsafe code therefore lives only in dependencies (tokio, rustls, ort, slint, …), which is what
  the supply-chain controls are for.

## 3. Licence audit (offline, against `deny.toml`)

Raw output: [`evidence/p10/licence-audit.txt`](evidence/p10/licence-audit.txt). Every third-party
package in the resolved graph is covered by the allow list or its crate exception (Slint under its
royalty-free licence, ADR-014), with one exception:

- `terminfo 0.9.0` — **WTFPL**, reached only through ratatui's optional `termwiz` backend
  (`ratatui-termwiz` → `termwiz` → `terminfo`). `cargo tree -i terminfo --target all` prints
  nothing: the feature is not enabled, so the crate is in the lock file but not in any build. If
  the termwiz backend is ever enabled, WTFPL needs an explicit decision (it is permissive but not
  OSI-approved).

## 4. What remains for P10-2

- `cargo deny check` (advisories, bans, sources) and `cargo audit`: need `cargo-deny`/`cargo-audit`
  installed and the RustSec database downloaded.
- `cargo vet`: needs `cargo-vet`; `supply-chain/audits.toml` holds no first-party audits yet.
- An independent review of the daemon and web surfaces.

## 5. Findings fixed during this phase

| Finding | Severity | Fix |
|---|---|---|
| An attached frontend could drop a message when its `select!` cancelled `RemoteSession::next_record` mid-read | reliability | pending-record slot; regression test fails on the old code (E53) |
| Blocking file read inside async `polaris web --connect` | low | `tokio::fs` (found by `/hack`, E55) |
| `/hack` first-version false positives (11 race candidates, 9 generic secret matches in polaris-kernel) | quality | data-flow requirement for races; precise credential rules for source (E55) |
