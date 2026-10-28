# S1 host follow-up

Verification status: **FOCUSED TESTS VERIFIED; STRICT CLIPPY PENDING**. The final configured-host integration target passed against the current S1 host and configured-frame source.

- The first invocation’s `Tee-Object` output-path error is preserved in `configured-host-tests-launch-error.txt`; that transcript is not a test result.
- The properly captured run `configured-host-tests.txt` compiled the earlier target and passed invalid-config rejection, authenticated startup, named-pipe connection rejection, and optional-PII-unconfigured behavior. It then failed because `taskkill` returned `ERROR: Access denied` for the worker PID. The test now terminates and checks the exact retained process tree through the kernel owner.
- Helper run `01-configured-host.txt` exited 101 before the host test binary ran. It failed compiling concurrent `polaris-security-rules/src/supply_chain.rs` edits at lines 342 and 369 (`&String` / `&str` mismatch).
- Helper run `02-configured-host.txt` exited 101 before compiling the host target. It failed compiling concurrent `RateLimitError::Busy` changes in `polaris-search/crates/providers/src/runtime.rs:145` and `polaris-nebius-cloud/crates/token-factory/src/client.rs:2102`.
- Helper run `03-configured-host.txt` compiled the integration target and executed all three tests, then exited 101 (`1 passed; 2 failed`). The invalid-configuration test passed. The worker lifecycle test observed tonic's generic `transport error` instead of a stable process-identity rejection; the frame test returned `Execution(TokenBudget)` because its 1,000-token fixture budget could not fund two turns at the tokenizer/output bounds. The client now reports a sanitized typed PID mismatch, the frame budget is 10,000 tokens, and the unused test import was removed. These fixes are not yet verified by a rerun.
- Helper run `04-configured-host.txt` passed all three configured-host tests after those fixes. The later final run `07-configured-host-final.txt` also passed all three after the configured outcome began retaining its actual session/frame-bound privacy vault. Both transcripts report `3 passed; 0 failed`.
- Helper run `05-normal-client.txt` passed all 35 `polaris-system-one-client` tests. Helper run `06-consumer-policy.txt` passed all 8 `polaris-system-one` consumer policy tests, including model-ready recovery and in-flight generation fencing.
- Current host tests check the exact named-pipe PID mismatch, restart and shutdown process-record retirement, actual readonly tool execution, tokenizer/provider masking, observed zero NER and positive degraded DFA fallback, and an `Unverified` completion without same-frame proof. These assertions remain unverified.
- Strict Clippy is still pending. The scoped rustfmt/descriptor/source scan results are recorded in `source-checks.txt`; the text scan is not a substitute for Clippy.

`commands.json` is the append-only command ledger for helper runs.
