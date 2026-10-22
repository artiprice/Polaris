#!/usr/bin/env bash
# P1's six mandatory parser targets, with bounded native Linux executions and retained logs.
set -euo pipefail
source "$HOME/.cargo/env"
cd "${1:-/mnt/c/mDev/Polaris}"
export CARGO_TARGET_DIR="$HOME/.cache/polaris-fuzz-target"
export CARGO_BUILD_JOBS=2
mkdir -p @prime/tmp/p1/fuzz-current
for target in directives_grammar rtk_stream cas_short_ref_roundtrip eventlog_decode proto_decode diff_apply; do
  logfile="@prime/tmp/p1/fuzz-current/$target.txt"
  echo "START $target"
  cargo +nightly fuzz run --target x86_64-unknown-linux-gnu --fuzz-dir tests/fuzz "$target" -- -max_total_time=60 -timeout=10 -rss_limit_mb=2048 -max_len=131072 >"$logfile" 2>&1
  tail -n 4 "$logfile"
  echo "PASS $target"
done
echo 'P1 mandatory fuzz targets: PASS'
