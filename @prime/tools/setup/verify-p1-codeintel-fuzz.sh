#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd /mnt/c/mDev/Polaris
export CARGO_TARGET_DIR="$HOME/.cache/polaris-fuzz-target"
export CARGO_BUILD_JOBS=2
for target in code_intel_parse code_intel_skeleton code_intel_merge3; do
  corpus="$HOME/.cache/polaris-$target-corpus"
  mkdir -p "$corpus"
  cargo +nightly fuzz run --target x86_64-unknown-linux-gnu --fuzz-dir tests/fuzz "$target" "$corpus" -- -max_total_time=60 -timeout=10 -rss_limit_mb=2048 -max_len=32768 >"@prime/tmp/p1/fuzz-current/$target.txt" 2>&1
  tail -n 3 "@prime/tmp/p1/fuzz-current/$target.txt"
  echo "PASS $target"
done
