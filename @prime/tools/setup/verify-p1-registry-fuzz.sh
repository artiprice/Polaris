#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd "${1:-/mnt/c/mDev/Polaris}"
export CARGO_TARGET_DIR="$HOME/.cache/polaris-fuzz-target"
export CARGO_BUILD_JOBS=2
for target in registry_fts_query registry_migration_open; do
  logfile="@prime/tmp/p1/fuzz-current/$target.txt"
  echo "START $target"
  cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu "$target" -- -max_total_time=60 -timeout=10 -rss_limit_mb=2048 -max_len=32768 >"$logfile" 2>&1
  tail -n 4 "$logfile"
  echo "PASS $target"
done
