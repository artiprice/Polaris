# Apply the reviewed Phase 2 reference-memory setting without replacing existing user config.
[CmdletBinding()]
param([switch]$Apply, [switch]$Restart)
$ErrorActionPreference = 'Stop'
if ($Restart -and -not $Apply) { throw '-Restart requires -Apply.' }
$configPath = Join-Path $env:USERPROFILE '.wslconfig'
$proposalPath = Join-Path $PSScriptRoot 'reference.wslconfig'
$proposal = [IO.File]::ReadAllText($proposalPath).Replace("`r`n", "`n")
if ($proposal -ne "[wsl2]`nmemory=6GB`n") { throw 'The reviewed proposal changed; inspect it before applying.' }
$alreadyConfigured = $false
if (Test-Path -LiteralPath $configPath) {
    $current = [IO.File]::ReadAllText($configPath).Replace("`r`n", "`n")
    if ($current -ne $proposal) {
        throw "Existing $configPath must be reviewed and merged; this script never overwrites it."
    }
    $alreadyConfigured = $true
}
Write-Output "REFERENCE_MEMORY_TARGET $configPath"
Write-Output $proposal.TrimEnd()
if (-not $Apply) {
    Write-Output 'PREVIEW_ONLY: no configuration or running WSL instance changed.'
    exit 0
}
if (-not $alreadyConfigured) {
    # CreateNew excludes a concurrent config creation; no user file is replaced.
    $stream = [IO.File]::Open($configPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($proposal)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
    Write-Output 'REFERENCE_MEMORY_CONFIG_WRITTEN memory=6GB'
} else {
    Write-Output 'REFERENCE_MEMORY_CONFIG_ALREADY_PRESENT memory=6GB'
}
if ($Restart) {
    & wsl.exe --shutdown
    if ($LASTEXITCODE -ne 0) { throw "WSL shutdown failed with exit $LASTEXITCODE; config is retained." }
    Write-Output 'WSL_SHUTDOWN_COMPLETE: memory cap will apply when WSL next starts.'
} else {
    Write-Output 'RESTART_PENDING: existing WSL instances retain their current memory allocation.'
}
