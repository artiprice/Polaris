#!/usr/bin/env bash
# Polaris P0-ENV-2 smoke test: OpenShell gateway in WSL2 Ubuntu (docker driver).
# Creates a detached sandbox, execs in it, checks exit-code propagation and
# byte-exact upload, then deletes it. Prints timings in milliseconds.
# Usage (from Windows):  wsl -d Ubuntu-24.04 -- bash /mnt/c/<repo>/@prime/tools/setup/verify-openshell.sh
set -uo pipefail

NAME="polaris-smoke-$$"
now_ms() { date +%s%3N; }
fail=0

# WSL2 stops idle distros; the gateway user service restarts with the distro (W14).
t=$(now_ms)
for _ in $(seq 1 60); do
  openshell status 2>/dev/null | grep -q 'Status: Connected' && break
  sleep 1
done
echo "== openshell $(openshell --version 2>/dev/null) / gateway: $(systemctl --user is-active openshell-gateway) (ready after $(( $(now_ms) - t )) ms)"
openshell status 2>&1 | grep -E 'Status|Version|Authentication' || true

t=$(now_ms)
if openshell sandbox create --name "$NAME" --detach --no-tty -- sleep infinity >/tmp/polaris-create.log 2>&1; then
  echo "create: ok ($(( $(now_ms) - t )) ms)"
else
  echo "create: FAILED"; tail -5 /tmp/polaris-create.log; exit 1
fi

phase=""
for _ in $(seq 1 60); do
  phase=$(openshell sandbox get "$NAME" -o json 2>/dev/null | jq -r '.phase // .status.phase // .sandbox.phase // empty' 2>/dev/null)
  [ "$phase" = "Ready" ] && break
  sleep 1
done
echo "phase: ${phase:-unknown}"

t=$(now_ms)
openshell sandbox exec -n "$NAME" --no-tty --no-login-shell -- \
  sh -c 'uname -sr; id; head -1 /etc/os-release; echo "landlock_abi=$(cat /sys/kernel/security/landlock/abi_version 2>/dev/null || echo n/a)"; echo "pwd=$(pwd)"' \
  || fail=1
echo "exec: $(( $(now_ms) - t )) ms"

openshell sandbox exec -n "$NAME" --no-tty --no-login-shell -- sh -c 'exit 7'
code=$?
if [ "$code" -eq 7 ]; then echo "exit-code propagation: ok (7)"; else echo "exit-code propagation: FAILED (got $code)"; fail=1; fi

tmp=$(mktemp -d)
printf 'hello\r\npolaris\n\x00\xff' > "$tmp/bytes.bin"
local_sum=$(sha256sum "$tmp/bytes.bin" | cut -d' ' -f1)
if openshell sandbox upload "$NAME" "$tmp/bytes.bin" /tmp/bytes.bin >/tmp/polaris-upload.log 2>&1; then
  remote_sum=$(openshell sandbox exec -n "$NAME" --no-tty --no-login-shell -- sha256sum /tmp/bytes.bin 2>/dev/null | cut -d' ' -f1)
  if [ "$local_sum" = "$remote_sum" ]; then echo "upload byte-exact: ok ($local_sum)"; else echo "upload byte-exact: FAILED local=$local_sum remote=$remote_sum"; fail=1; fi
else
  echo "upload: FAILED"; tail -5 /tmp/polaris-upload.log; fail=1
fi
rm -rf "$tmp"

openshell sandbox delete "$NAME" >/dev/null 2>&1 || fail=1
for _ in $(seq 1 30); do
  openshell sandbox get "$NAME" >/dev/null 2>&1 || break
  sleep 1
done
if openshell sandbox get "$NAME" >/dev/null 2>&1; then echo "delete: FAILED (still present)"; fail=1; else echo "delete: ok"; fi

[ "$fail" -eq 0 ] && echo "RESULT: PASS" || echo "RESULT: FAIL"
exit "$fail"
