#!/usr/bin/env bash
# One-shot OpenShell bring-up on this machine: wait for the gateway, run the smoke test, then
# build the Polaris sandbox image. Run as the distro's default user (in the docker group):
#   wsl -d Ubuntu -- bash /mnt/d/Polaris/@prime/tools/sandbox/bringup-wsl.sh /mnt/d/Polaris
set -uo pipefail
ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}"
tmp="$(mktemp -d)"
for script in "$ROOT/@prime/tools/setup/verify-openshell.sh" "$ROOT/@prime/tools/sandbox/build-wsl.sh"; do
  sed 's/\r$//' "$script" > "$tmp/$(basename "$script")"
done
echo "== smoke test"
bash "$tmp/verify-openshell.sh" || { echo "smoke test failed"; exit 1; }
echo "== sandbox image"
POLARIS_ROOT="$ROOT" bash "$tmp/build-wsl.sh" 2>&1 | tail -n 40
rm -rf "$tmp"
