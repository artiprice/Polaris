#!/usr/bin/env sh
# Where the local OpenShell gateway keeps its client mTLS files, and what it reports.
# Usage (from Windows): wsl -d Ubuntu -- sh /mnt/d/Polaris/@prime/tools/setup/openshell-facts.sh
openshell --version
openshell status 2>&1 | head -8
find "$HOME/.config/openshell" -maxdepth 6 \( -name '*.crt' -o -name '*.key' -o -name '*.pem' \) 2>/dev/null
docker version --format 'docker server {{.Server.Version}}' 2>&1 | tail -1
