#!/usr/bin/env bash
# S0-12 follow-up: what does the default OpenShell sandbox policy allow? Prints the effective policy
# and probes exec/write locations used by the browser image. Leaves the probe sandbox running if
# KEEP=1 (for iteration); deletes it otherwise.
set -uo pipefail
N=polaris-s0-12p
IMG=${IMG:-polaris/s0-12:dev}
x() { openshell sandbox exec -n "$N" --no-tty --no-login-shell -- "$@"; }
if ! openshell sandbox get "$N" >/dev/null 2>&1; then
  openshell sandbox create --name "$N" --from "$IMG" --detach --no-tty -- sleep infinity >/dev/null 2>&1
fi
for _ in $(seq 1 120); do
  p=$(openshell sandbox get "$N" -o json 2>/dev/null | jq -r '.phase // .status.phase // .sandbox.phase // empty' 2>/dev/null)
  [ "$p" = "Ready" ] && break
  sleep 0.5
done
echo "phase=$p"
echo "--- effective policy"
openshell policy get "$N" 2>&1 | head -120
echo "--- probes"
x sh -c '
  ls -l /opt/chrome-linux64/chrome
  /opt/chrome-linux64/chrome --version >/dev/null 2>&1; echo "exec /opt/...chrome rc=$?"
  /usr/bin/Xvfb -help >/dev/null 2>&1; echo "exec /usr/bin/Xvfb rc=$?"
  touch /tmp/t 2>/dev/null && echo "write /tmp ok" || echo "write /tmp DENIED"
  touch /sandbox/t 2>/dev/null && echo "write /sandbox ok" || echo "write /sandbox DENIED"
  mkdir -p /tmp/.X11-unix 2>/dev/null && echo "mkdir /tmp/.X11-unix ok" || echo "mkdir /tmp/.X11-unix DENIED"
  ls -ld /tmp/.X11-unix /dev/shm 2>&1
  cat /proc/self/status | grep -E "^(NoNewPrivs|Seccomp):"
'
[ "${KEEP:-0}" = "1" ] || { openshell sandbox delete "$N" >/dev/null 2>&1; echo "probe sandbox deleted"; }
