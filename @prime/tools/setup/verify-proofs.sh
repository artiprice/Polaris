#!/usr/bin/env bash
# Miri + Kani + cargo-fuzz for the implemented crates, inside WSL2 (per-PC installs `wsl-rust`,
# `wsl-kani`).
#   wsl -d Ubuntu-24.04 -- bash @prime/tools/setup/verify-proofs.sh [fuzz-seconds-per-target]   (default 60)
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
FUZZ_SECONDS="${1:-60}"
# Miri: pure crates only. Excluded: polaris-storage-cas (zstd C code through FFI) and polaris-storage-eventlog (redb file I/O),
# which Miri cannot execute.
MIRI_CRATES=(polaris-sdk polaris-kernel-directives polaris-kernel-bus)
# Kani: only harnesses CBMC can finish within WSL memory (PLAN W24). polaris-storage-cas has none yet:
# a fold_ref harness grew CBMC past 6 GB.
KANI_CRATES=(polaris-sdk polaris-kernel-directives polaris-security-capability)
FUZZ_TARGETS=(directives_grammar cas_blob_decode eventlog_decode config_toml_layer rtk_stream ui_completion biscuit_token_bytes)
# Tests skipped under Miri, by name, with the reason:
#   event_record_json_snapshot — insta calls socketpair(type 5), which Miri does not support
#   (an unsupported operation in the dev-dependency, not undefined behaviour in our code).
MIRI_SKIP=(--skip event_record_json_snapshot)
# cargo-fuzz defaults --target to the triple its own binary was built for; cargo-binstall installs the
# musl build, so pin glibc (AddressSanitizer needs it, and the musl std is not installed).
FUZZ_TRIPLE=x86_64-unknown-linux-gnu
log="$(mktemp)"
fail=0

for crate in "${MIRI_CRATES[@]}"; do
  echo "== miri: $crate"
  CARGO_TARGET_DIR="$HOME/.cache/polaris-target-miri" PROPTEST_CASES=8 MIRIFLAGS="-Zmiri-disable-isolation" \
    cargo +nightly miri test -p "$crate" --tests --no-fail-fast -- "${MIRI_SKIP[@]}" >"$log" 2>&1
  rc=$?
  grep -E "test result|Undefined Behavior|unsupported operation|panicked" "$log"
  echo "miri $crate exit=$rc"
  [ "$rc" -eq 0 ] || fail=1
done

for crate in "${KANI_CRATES[@]}"; do
  echo "== kani: $crate"
  if command -v cargo-kani >/dev/null 2>&1; then
    # Memory-guarded (CBMC stopped above 3 GiB) so an exploding harness cannot OOM the VM.
    bash "$(dirname "$0")/kani-guarded.sh" "$crate" | grep -vE "^\s+- Description"
    rc=${PIPESTATUS[0]}
    [ "$rc" -eq 0 ] || fail=1
  else
    echo "kani not installed (setup-wsl.sh --with-kani)"; fail=1
  fi
done

for target in "${FUZZ_TARGETS[@]}"; do
  echo "== fuzz: $target (${FUZZ_SECONDS}s)"
  CARGO_TARGET_DIR="$HOME/.cache/polaris-target-fuzz" \
    cargo +nightly fuzz run --fuzz-dir tests/fuzz --target "$FUZZ_TRIPLE" "$target" -- -max_total_time="$FUZZ_SECONDS" >"$log" 2>&1
  rc=$?
  grep -E "^#[0-9]+[[:space:]]+(INITED|DONE)|^Done [0-9]+ runs|SUMMARY|panicked|ERROR|error\[" "$log" | tail -6
  echo "fuzz $target exit=$rc"
  [ "$rc" -eq 0 ] || fail=1
done

rm -f "$log"
[ "$fail" -eq 0 ] && echo "RESULT: PASS" || echo "RESULT: FAIL"
exit "$fail"
