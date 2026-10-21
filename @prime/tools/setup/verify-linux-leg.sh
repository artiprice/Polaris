#!/usr/bin/env bash
# Polaris P0-ENV-5: Linux build leg inside WSL2 (keeps non-Windows code from rotting).
# Uses a Linux-only target dir so it never clobbers the Windows `target/`.
# Usage: wsl -d Ubuntu-24.04 -- bash /mnt/c/<repo>/@prime/tools/setup/verify-linux-leg.sh /mnt/c/<repo>
set -euo pipefail
REPO="${1:-/mnt/c/mDev/Polaris}"
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
export CARGO_TARGET_DIR="$HOME/.cache/polaris-target"
cd "$REPO"
python3 -I @prime/tools/check-vendored-biscuit.py
echo "toolchain: $(rustc --version) | nightly: $(rustc +nightly --version) | nextest: $(cargo nextest --version | head -1) | fuzz: $(cargo fuzz --version) | kani: $(cargo kani --version 2>/dev/null | head -1)"
RUSTFLAGS="-Dwarnings" cargo check --workspace --all-targets 2>&1 | tail -2
RUSTDOCFLAGS="-Dwarnings" cargo doc --workspace --no-deps 2>&1 | tail -1
cargo fmt --all --check && echo "fmt: ok"
echo "packages: $(cargo metadata --format-version 1 --no-deps | jq '.packages | length')"
echo "RESULT: PASS"
