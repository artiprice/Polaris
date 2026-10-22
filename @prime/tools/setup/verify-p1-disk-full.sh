#!/usr/bin/env bash
set -euo pipefail
source "$HOME/.cargo/env"
cd /mnt/c/mDev/Polaris
export CARGO_TARGET_DIR="$HOME/.cache/polaris-target"
export CARGO_BUILD_JOBS=2
export CAS_TEST_FULL_DIRECTORY=/mnt/c/mDev/Polaris/@prime/tmp/p1/disk-full
cargo test -p polaris-storage-cas --test disk_full disk_full_leaves_no_partial_blob -- --ignored --nocapture
