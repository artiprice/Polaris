#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd /mnt/c/mDev/Polaris
for crate in polaris-sdk polaris-kernel-directives polaris-code-intel; do
  bash @prime/tools/setup/kani-guarded.sh "$crate" >"@prime/tmp/p1/$crate-kani-final.txt" 2>&1
  tail -n 5 "@prime/tmp/p1/$crate-kani-final.txt"
done
export CARGO_TARGET_DIR="$HOME/.cache/polaris-target-miri"
export PROPTEST_CASES=8
export MIRIFLAGS=-Zmiri-disable-isolation
# Loom's generator uses getrlimit/context switching, unsupported by Miri. The allocation checker
# instruments the native allocator. Both remain ordinary native tests; interpret the 13 bus tests.
cargo +nightly miri test -p polaris-kernel-bus --test command_backpressure --test lag_recovery --test log_then_publish >@prime/tmp/p1/bus-miri-supported-final.txt 2>&1
tail -n 8 @prime/tmp/p1/bus-miri-supported-final.txt
