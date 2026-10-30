#!/usr/bin/env sh
# Start a local SearXNG for `/search-web` and `$websearch` (P5-X-1) on Linux or WSL2 with
# Docker. Same container as run.ps1: loopback only, minimal capabilities, settings copy with a
# fresh secret key under ~/.polaris/searxng.
#
#   sh @prime/tools/searxng/run.sh          # start (or restart) it
#   sh @prime/tools/searxng/run.sh stop     # stop and remove it
set -eu
image="${SEARXNG_IMAGE:-searxng/searxng:latest}"
port="${SEARXNG_PORT:-8888}"
name="polaris-searxng"

docker rm -f "$name" >/dev/null 2>&1 || true
if [ "${1:-}" = "stop" ]; then
    echo "Stopped $name."
    exit 0
fi

config="$HOME/.polaris/searxng"
mkdir -p "$config"
secret="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
sed "s/REPLACED_AT_RUN/$secret/" "$(dirname "$0")/settings.yml" > "$config/settings.yml"
chmod 600 "$config/settings.yml"

docker run -d --name "$name" --restart unless-stopped \
    -p "127.0.0.1:$port:8080" \
    -v "$config:/etc/searxng" \
    --cap-drop ALL --cap-add CHOWN --cap-add SETGID --cap-add SETUID \
    --security-opt no-new-privileges \
    "$image" >/dev/null
echo "SearXNG is starting on http://127.0.0.1:$port (container $name)."
