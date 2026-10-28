# E48 — Phase 3 implementation checkpoint

VERIFIED scoped implementation progress. **Phase 3 is not complete.** UI deferred; P2 paused.

- Durable SDK/proto human protocol, separate verifier authority, real host bus answer dispatch and CAS evidence/replay: seven kernel cases plus SDK/proto contracts.
- Real workspace write/edit/patch, path policy, Biscuit facts, leases/OCC, hybrid verification and durable receipts: eleven cases with **synthetic S1 assessment**, not production calibration.
- Stateless Contree MCP inventory/run/wait/cancel/branch/file/image and approved-path uploads: six **synthetic client** contracts. Actual MCP hub registry adapter preserves caller effect authorization; deployed Contree integration is not proved by these fixtures.
- Event remains within its existing 96-byte limit; answer audit metadata lives in CAS. Schema v1-v4 compatibility and future-version rejection remain.
- Windows fixture failure reproduced as Winsock WouldBlock on deliberately fragmented requests; accepted sockets now use explicit blocking mode under existing timeouts. The original composition assertions remain, plus a new fragmentation assertion; 20 executions of both cases passed.

## Evidence

`final-regression` in [commands.json](commands.json) runs SDK, proto, kernel, kernel-verifier, code-intel, Contree, MCP, UI model compatibility and telemetry: **583 passed, 0 failed, 3 ignored**. `final-workspace` compiles every workspace target. `boundaries-six-safety` covers six libraries only; `kernel-safety-current` also checks the two new integration suites. Member/layer/witness, scoped format and generator ownership gates pass. See raw command files for exact scope; this is not strict whole-workspace acceptance.

Ignored workloads remain the code-intel release throughput case, 1000-directory/20-process teardown, and 200-process worker-kill stress. No new Linux, Miri, Kani, keyed, full-load or dependency audit acceptance was established here.

Earlier failures are retained, including the compact-event regression, stale-answer bug, source-read bounds, exact newline reconstruction, role denial, cancellation after verification, non-durable witnesses and fragmented HTTP. [Focused RED/GREEN logs](retained-review-logs.json) are supplemental; initial agent API/build blockers do not establish universal test-first development.

## Still required

Model-visible mutation/ask_user/run_tests dispatch; actual frame-tree cancellation and sibling/requester continuation; independent proof tied to the final current source tree; exact local model/tokenizer pairing and live M3 transcript; Contree sidecar deployment, SWE resolver and live workload/security gates; all remaining owner matrices in [P3 checklist](../../P3_COMPLETION_CHECKLIST.md). Token Factory and Contree access is unavailable, so M4/P3-X-2/3/4 live checks remain blocked on access.

The expanded Code Intel safety check remains RED with **38 arithmetic violations** in existing decl/merge/metrics/parser/repomap/skeleton code. Broader dependency safety also found existing log/storage-sync conversions. These failures were not suppressed or reclassified as passing. Rust syntax coverage does not prove semantic safety; macros/attributes remain incomplete. The mutation service assumes a stable workspace/cooperating lease owners and rejects unsupported newline conversions; hostile directory replacement, new directories, complete revocation, fault/stress/formal acceptance and calibrated production S1 are not established. Raw source and sensitive payloads can occupy owned buffers; no complete DLP/zero-allocation claim is made.

Contree sync currently rejects deletes/conflicts and caps files at 700,000 bytes, batches at 16 MiB/256 files. Remote cancellation requires successful operation reconciliation; dropping a caller future is not proved durable remote recovery. Actual timeout/failure recovery and secret/cost acceptance require deployment evidence. RPC explicitly rejects answer commands until a dedicated remote host authority path exists; the local trusted host API is implemented. Visual UI work remains deferred.

## Consistency

PLAN corrects the old persistent SandboxBackend/18-tool sketch against [pinned upstream](https://github.com/nebius/contree-mcp/tree/c2f1541e3993bc77c3c7dbcf57b0edfe5b3634cc). Metadata/generated phase tables, SDK/proto, projections, RPC compatibility and percentage estimates are updated. Percentages remain engineering estimates, not phase gates. The generator preservation bug was reproduced and fixed; exact unchanged tracked test-package files were restored and preservation rechecked. D10 is followed using external lint checks, with no new source lint attributes. Initial automatic rejection of removing newly introduced attributes was resolved with original-source evidence and identical external enforcement.
