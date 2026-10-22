#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd /mnt/c/mDev/Polaris
export CARGO_TARGET_DIR="$HOME/.cache/polaris-target"
export CARGO_BUILD_JOBS=2
export EVENTLOG_TEST_FULL_DIRECTORY=/mnt/c/mDev/Polaris/@prime/tmp/p1/disk-full
cargo test -p polaris-storage-eventlog --test disk_full append_failure_preserves_committed_prefix -- --ignored --nocapture
