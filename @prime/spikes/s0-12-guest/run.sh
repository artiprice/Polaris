#!/usr/bin/env bash
# Spike S0-12 (run inside WSL2 as the default user):
#   guest build in WSL2 -> sandbox image -> OpenShell sandbox from it -> protocol handshake ->
#   headless Chrome screenshot -> headed Chrome under Xvfb + root-window dump -> cleanup.
#   wsl -d Ubuntu-24.04 -- bash /mnt/c/<repo>/@prime/spikes/s0-12-guest/run.sh
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/out"
ctx="$HOME/.cache/polaris-s0-12-ctx"
NAME=polaris-s0-12
IMG=polaris/s0-12:dev
now_ms() { date +%s%3N; }
x() { openshell sandbox exec -n "$NAME" --no-tty --no-login-shell -- "$@"; }
rm -rf "$out" "$ctx" && mkdir -p "$out" "$ctx"
# shellcheck disable=SC1091
. "$HOME/.cargo/env"

echo "== 1 guest build in WSL2"
t=$(now_ms)
CARGO_TARGET_DIR="$HOME/.cache/polaris-s0-12-target" cargo build --release -q --manifest-path "$here/guest/Cargo.toml" || exit 1
cp "$HOME/.cache/polaris-s0-12-target/release/polaris-guest" "$ctx/"
echo "guest: $(file -b "$ctx/polaris-guest" | cut -c1-90); $(stat -c %s "$ctx/polaris-guest") B; $(( $(now_ms) - t )) ms"

echo "== 2 sandbox image"
cft=$(curl -fsSL https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions.json | jq -r .channels.Stable.version)
echo "chrome-for-testing stable: $cft"
cp "$here/image/Dockerfile" "$here/image/test.html" "$ctx/"
t=$(now_ms)
docker build -q --build-arg CFT_VERSION="$cft" -t "$IMG" "$ctx" >"$out/build.log" 2>&1 || { tail -20 "$out/build.log"; exit 1; }
echo "image: $IMG size=$(docker image inspect "$IMG" --format '{{.Size}}') B id=$(docker image inspect "$IMG" --format '{{.Id}}' | cut -c1-19) build=$(( $(now_ms) - t )) ms"

echo "== 3 sandbox from the image"
openshell sandbox delete "$NAME" >/dev/null 2>&1
t=$(now_ms)
openshell sandbox create --name "$NAME" --from "$IMG" --detach --no-tty -- sleep infinity >"$out/create.log" 2>&1 || { cat "$out/create.log"; exit 1; }
phase=""
for _ in $(seq 1 240); do
  phase=$(openshell sandbox get "$NAME" -o json 2>/dev/null | jq -r '.phase // .status.phase // .sandbox.phase // empty' 2>/dev/null)
  [ "$phase" = "Ready" ] && break
  sleep 0.5
done
echo "create -> ${phase:-?} in $(( $(now_ms) - t )) ms"

echo "== 4 protocol handshake"
x polaris-guest handshake 1; echo "handshake(host=1) exit=$?"
x polaris-guest handshake 2; echo "handshake(host=2) exit=$? (3 = mismatch refused)"
x sh -c 'echo "user=$(id -un) home=$HOME pwd=$(pwd)"'

# Artifacts go under /sandbox: `openshell sandbox download` only serves the workspace (run 1).
x mkdir -p /sandbox/out

echo "== 5 headless Chrome screenshot"
t=$(now_ms)
x chrome --headless=new --no-sandbox --disable-gpu --disable-dev-shm-usage --window-size=800,600 \
  --screenshot=/sandbox/out/headless.png file:///opt/polaris/test.html >"$out/headless.log" 2>&1
echo "headless exit=$? ($(( $(now_ms) - t )) ms)"

echo "== 6 headed Chrome under Xvfb + root-window dump"
t=$(now_ms)
x sh -c '
  Xvfb :99 -screen 0 800x600x24 -nolisten tcp >/sandbox/out/xvfb.err 2>&1 &
  xpid=$!
  for i in $(seq 1 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.1; done
  ls -l /tmp/.X11-unix/ >>/sandbox/out/xvfb.err 2>&1
  DISPLAY=:99 chrome --no-sandbox --disable-gpu --disable-dev-shm-usage --no-first-run --kiosk \
    --window-size=800,600 file:///opt/polaris/test.html >/sandbox/out/headed.log 2>&1 &
  cpid=$!
  sleep 8
  xwd -display :99 -root -silent >/sandbox/out/xvfb.xwd; rc=$?
  kill $cpid $xpid 2>/dev/null
  exit $rc' >"$out/xvfb.log" 2>&1
echo "xvfb exit=$? ($(( $(now_ms) - t )) ms)"

echo "== 7 artifacts"
for f in headless.png xvfb.xwd headed.log xvfb.err; do
  openshell sandbox download "$NAME" "/sandbox/out/$f" "$out/" >>"$out/download.log" 2>&1 || echo "download $f failed"
done
ls -l "$out" | tail -n +2
python3 -I "$here/check_images.py" "$out"

echo "== 8 delete"
openshell sandbox delete "$NAME" >/dev/null 2>&1
for _ in $(seq 1 60); do openshell sandbox get "$NAME" >/dev/null 2>&1 || break; sleep 0.5; done
if openshell sandbox get "$NAME" >/dev/null 2>&1; then echo "delete: still present"; else echo "delete: ok"; fi
