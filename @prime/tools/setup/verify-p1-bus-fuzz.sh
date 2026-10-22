#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd /mnt/c/mDev/Polaris
export CARGO_TARGET_DIR="$HOME/.cache/polaris-fuzz-target"
export CARGO_BUILD_JOBS=2
corpus="$HOME/.cache/polaris-bus-fuzz-corpus"
mkdir -p "$corpus"
cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu bus_ops "$corpus" -- -max_total_time=60 -timeout=10 -rss_limit_mb=2048 -max_len=4096 >@prime/tmp/p1/fuzz-current/bus_ops.txt 2>&1
tail -n 5 @prime/tmp/p1/fuzz-current/bus_ops.txt
