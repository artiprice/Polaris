#!/usr/bin/env bash
# S0-12 follow-up: try Chrome/Xvfb variants inside one sandbox from polaris/s0-12:dev.
set -uo pipefail
N=polaris-s0-12d
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/out-debug"; rm -rf "$out"; mkdir -p "$out"
x() { openshell sandbox exec -n "$N" --no-tty --no-login-shell -- "$@"; }
openshell sandbox delete "$N" >/dev/null 2>&1
openshell sandbox create --name "$N" --from polaris/s0-12:dev --detach --no-tty -- sleep infinity >/dev/null 2>&1
for _ in $(seq 1 120); do
  p=$(openshell sandbox get "$N" -o json 2>/dev/null | jq -r '.phase // .status.phase // .sandbox.phase // empty' 2>/dev/null)
  [ "$p" = "Ready" ] && break; sleep 0.5
done
echo "phase=$p"
x mkdir -p /sandbox/out

echo "== A headless, --no-zygote"
x sh -c 'chrome --headless=new --no-sandbox --no-zygote --disable-gpu --disable-dev-shm-usage --window-size=800,600 --screenshot=/sandbox/out/a.png file:///opt/polaris/test.html >/sandbox/out/a.log 2>&1; echo "rc=$?"; grep -m3 -E "FATAL|Check failed|written to file" /sandbox/out/a.log'
echo "== B headless, --no-zygote --single-process"
x sh -c 'chrome --headless=new --no-sandbox --no-zygote --single-process --disable-gpu --disable-dev-shm-usage --window-size=800,600 --screenshot=/sandbox/out/b.png file:///opt/polaris/test.html >/sandbox/out/b.log 2>&1; echo "rc=$?"; grep -m3 -E "FATAL|Check failed|written to file" /sandbox/out/b.log'
echo "== C xkbcomp / xkb data"
x sh -c 'command -v xkbcomp; xkbcomp -version 2>&1 | head -1; echo "xkbcomp rc=$?"; ls /usr/share/X11/xkb 2>&1 | head -3; echo "TMPDIR=${TMPDIR:-unset}"'
echo "== D Xvfb with explicit TMPDIR + keymap compile to /tmp"
x sh -c 'mkdir -p /tmp/.X11-unix 2>/dev/null; chmod 1777 /tmp/.X11-unix 2>/dev/null; TMPDIR=/tmp Xvfb :99 -screen 0 800x600x24 -nolisten tcp >/sandbox/out/d.err 2>&1 & sleep 2; ls -l /tmp/.X11-unix; xwd -display :99 -root -silent > /sandbox/out/d.xwd; echo "xwd rc=$? bytes=$(stat -c %s /sandbox/out/d.xwd)"; tail -5 /sandbox/out/d.err'
for f in a.png b.png d.xwd a.log b.log d.err; do openshell sandbox download "$N" "/sandbox/out/$f" "$out/" >/dev/null 2>&1; done
ls -l "$out" | tail -n +2
openshell sandbox delete "$N" >/dev/null 2>&1; echo "deleted"
