#!/usr/bin/env sh
# What the OpenShell setup left in this WSL distro: installers still running, Docker, OpenShell
# and its gateway. Usage (from Windows): wsl -d Ubuntu -- sh /mnt/d/Polaris/@prime/tools/setup/wsl-status.sh
ps -eo pid,etime,cmd | grep -E 'apt|dpkg|curl|install.sh|setup-wsl' | grep -v grep | head -8
echo "docker: $(command -v docker || echo absent) $(docker version --format '{{.Server.Version}}' 2>/dev/null || sudo -n docker version --format '{{.Server.Version}}' 2>/dev/null || echo 'no server')"
echo "openshell: $(command -v openshell || echo absent) $(openshell --version 2>/dev/null)"
echo "gateway: $(systemctl --user is-active openshell-gateway 2>&1)"
