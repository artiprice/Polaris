#!/usr/bin/env sh
# Network reachability from this WSL distro (DNS and HTTPS to the hosts the OpenShell setup uses).
cat /etc/resolv.conf | grep -v '^#' | head -3
for host in archive.ubuntu.com security.ubuntu.com download.docker.com raw.githubusercontent.com github.com; do
  addr=$(getent hosts "$host" | awk '{print $1}' | head -1)
  code=$(curl -s -o /dev/null -m 8 -w '%{http_code}' "https://$host/" 2>/dev/null || echo fail)
  echo "$host dns=${addr:-none} https=$code"
done
grep -h '^URIs\|^deb ' /etc/apt/sources.list /etc/apt/sources.list.d/* 2>/dev/null | head -5
