#!/usr/bin/env bash
# Runs `cargo kani -p <crate> --features kani` with a memory guard: CBMC is stopped when its RSS
# exceeds the limit (default 3 GiB), so a harness that explodes cannot OOM the WSL2 VM (and the
# OpenShell gateway in it; PLAN W24).
#   wsl -d Ubuntu-24.04 -- bash @prime/tools/setup/kani-guarded.sh polaris-kernel-directives [limit_kib]
set -uo pipefail
cd "$(dirname "$0")/../../.." || exit 1
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
crate="${1:?usage: kani-guarded.sh <crate> [limit_kib]}"
limit_kib="${2:-3145728}"
log="$(mktemp)"

guard() {
  while sleep 10; do
    for pid in $(pgrep -x cbmc); do
      rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
      if [ "${rss:-0}" -gt "$limit_kib" ]; then
        echo "GUARD: cbmc pid $pid at ${rss} KiB > ${limit_kib} KiB, stopping it"
        kill "$pid"
      fi
    done
  done
}
guard &
guard_pid=$!
start=$(date +%s)
CARGO_TARGET_DIR="$HOME/.cache/polaris-target-kani" cargo kani -p "$crate" --features kani >"$log" 2>&1
rc=$?
kill "$guard_pid" 2>/dev/null
grep -E "^Checking harness|^VERIFICATION:|^Complete -|^Verification Time|^Failed Checks|unwinding assertion" "$log"
echo "kani $crate exit=$rc wall_s=$(( $(date +%s) - start ))"
rm -f "$log"
exit "$rc"
