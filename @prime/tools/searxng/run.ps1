# Start a local SearXNG for `/search-web` and `$websearch` (P5-X-1).
#
# Needs Docker Desktop; the first run pulls the image. The container listens on 127.0.0.1 only,
# drops every capability except the three its entrypoint needs, and reads a settings copy under
# ~/.polaris/searxng with a fresh secret key (the repository file keeps a placeholder).
#
#   powershell -File @prime/tools/searxng/run.ps1            # start (or restart) it
#   powershell -File @prime/tools/searxng/run.ps1 -Stop      # stop and remove it
#
# Polaris finds it at http://127.0.0.1:8888 (override with POLARIS_SEARXNG_URL).
param(
    [string]$Image = "searxng/searxng:latest",
    [int]$Port = 8888,
    [switch]$Stop
)
$ErrorActionPreference = "Stop"
$name = "polaris-searxng"

docker rm -f $name 2>$null | Out-Null
if ($Stop) { Write-Output "Stopped $name."; return }

$config = Join-Path $env:USERPROFILE ".polaris\searxng"
New-Item -ItemType Directory -Force $config | Out-Null
$bytes = New-Object byte[] 32
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
$secret = -join ($bytes | ForEach-Object { $_.ToString("x2") })
$template = Get-Content -Raw (Join-Path $PSScriptRoot "settings.yml")
$settings = $template.Replace("REPLACED_AT_RUN", $secret)
[System.IO.File]::WriteAllText((Join-Path $config "settings.yml"), $settings)

docker run -d --name $name --restart unless-stopped `
    -p "127.0.0.1:${Port}:8080" `
    -v "${config}:/etc/searxng" `
    --cap-drop ALL --cap-add CHOWN --cap-add SETGID --cap-add SETUID `
    --security-opt no-new-privileges `
    $Image | Out-Null
Write-Output "SearXNG is starting on http://127.0.0.1:$Port (container $name)."
