#!/usr/bin/env bash
# S0-2: OpenShell `vm` compute driver (libkrun microVMs) under nested KVM in WSL2.
# Switches the local gateway to the vm driver, runs the smoke test, then restores the
# docker driver (pinned explicitly, which also removes the boot-time auto-detect race).
set -uo pipefail
ENV_FILE="$HOME/.config/openshell/gateway.env"
SMOKE="/mnt/c/mDev/Polaris/@prime/tools/setup/verify-openshell.sh"
mkdir -p "$(dirname "$ENV_FILE")"

set_driver() {
  if [ -f "$ENV_FILE" ] && grep -q '^OPENSHELL_COMPUTE_DRIVER=' "$ENV_FILE"; then
    sed -i "s/^OPENSHELL_COMPUTE_DRIVER=.*/OPENSHELL_COMPUTE_DRIVER=$1/" "$ENV_FILE"
  else
    echo "OPENSHELL_COMPUTE_DRIVER=$1" >> "$ENV_FILE"
  fi
  systemctl --user restart openshell-gateway
  for _ in $(seq 1 60); do openshell status 2>/dev/null | grep -q 'Status: Connected' && break; sleep 1; done
  journalctl --user -u openshell-gateway --no-pager -n 30 | grep -Ei 'Using compute driver|driver connected|error' | tail -3
}

echo "== switch to vm driver"
set_driver vm
echo "== smoke on vm driver"
start=$(date +%s)
bash "$SMOKE"; rc=$?
echo "vm smoke exit=$rc elapsed_s=$(( $(date +%s) - start ))"
echo "== gateway log (vm)"; journalctl --user -u openshell-gateway --no-pager -n 80 | grep -Ei 'vm|krun|kvm|error|warn' | tail -12

echo "== restore docker driver (pinned)"
set_driver docker
bash "$SMOKE" | tail -2
cat "$ENV_FILE"
exit "$rc"
