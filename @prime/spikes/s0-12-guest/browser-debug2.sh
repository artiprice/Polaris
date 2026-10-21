#!/usr/bin/env bash
# S0-12 follow-up 2: page source location vs policy; Xvfb verbose keymap failure.
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
x sh -c 'mkdir -p /sandbox/out; cat /opt/polaris/test.html >/dev/null 2>&1 && echo "read /opt ok" || echo "read /opt DENIED"; cat /usr/lib/chrome-linux64/chrome_100_percent.pak >/dev/null 2>&1 && echo "read /usr ok" || echo "read /usr DENIED"; cp /usr/share/X11/xkb/rules/evdev /dev/null 2>&1 && echo "read /usr/share ok"'
x sh -c 'printf "%s" "<!doctype html><body style=\"margin:0;background:#76b900;font:48px sans-serif\"><h1>Polaris S0-12</h1><button>OK</button></body>" > /sandbox/out/test.html'
C='chrome --headless=new --no-sandbox --no-zygote --disable-gpu --disable-dev-shm-usage --window-size=800,600'
echo "== E headless, page from /sandbox"
x sh -c "$C --screenshot=/sandbox/out/e.png file:///sandbox/out/test.html >/sandbox/out/e.log 2>&1; echo rc=\$?; grep -m2 -E 'written to file|Page load failed|FATAL' /sandbox/out/e.log"
echo "== F headless, data: URL"
x sh -c "$C --screenshot=/sandbox/out/f.png 'data:text/html,<body style=\"margin:0;background:%2376b900\"><h1>F</h1></body>' >/sandbox/out/f.log 2>&1; echo rc=\$?; grep -m2 -E 'written to file|Page load failed|FATAL' /sandbox/out/f.log"
echo "== G Xvfb verbose"
x sh -c 'Xvfb :98 -screen 0 800x600x24 -nolisten tcp -verbose 7 -logverbose 7 >/sandbox/out/g.err 2>&1 & sleep 3; grep -iE -m8 "xkbcomp|xkm|keymap|error|Popen|exec" /sandbox/out/g.err; ls -la /tmp | grep -iE "xkm|server" ; true'
for f in e.png f.png e.log f.log g.err; do openshell sandbox download "$N" "/sandbox/out/$f" "$out/" >/dev/null 2>&1; done
ls -l "$out" | tail -n +2
openshell sandbox delete "$N" >/dev/null 2>&1; echo "deleted"
