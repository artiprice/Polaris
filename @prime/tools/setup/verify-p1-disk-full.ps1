# Mount only an empty, fixed test directory; the test itself runs as the ordinary WSL user.
$ErrorActionPreference = 'Stop'
$workspace = 'C:\mDev\Polaris'
$mountDirectory = Join-Path $workspace '@prime\tmp\p1\disk-full'
$linuxDirectory = '/mnt/c/mDev/Polaris/@prime/tmp/p1/disk-full'
$null = New-Item -ItemType Directory -Path $mountDirectory -Force
$resolvedDirectory = (Resolve-Path -LiteralPath $mountDirectory).Path
if (-not $resolvedDirectory.StartsWith($workspace + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Test mount escaped workspace' }
if ((Get-Item -LiteralPath $resolvedDirectory).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Test mount cannot be a reparse point' }
if (Get-ChildItem -LiteralPath $resolvedDirectory -Force) { throw 'Test mount directory must be empty' }
$mounted = $false
try {
    & wsl -d Ubuntu-24.04 -u root -- mount -t tmpfs -o size=65536,mode=1777,nosuid,nodev,noexec tmpfs $linuxDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Disposable tmpfs mount failed' }
    $mounted = $true
    & wsl -d Ubuntu-24.04 -- bash /mnt/c/mDev/Polaris/@prime/tools/setup/verify-p1-disk-full.sh
    if ($LASTEXITCODE -ne 0) { throw 'Disk-full CAS regression failed' }
    & wsl -d Ubuntu-24.04 -u root -- umount $linuxDirectory
    if ($LASTEXITCODE -ne 0) { throw 'CAS test mount cleanup failed' }
    $mounted = $false
    & wsl -d Ubuntu-24.04 -u root -- mount -t tmpfs -o size=16777216,mode=1777,nosuid,nodev,noexec tmpfs $linuxDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Eventlog test mount failed' }
    $mounted = $true
    & wsl -d Ubuntu-24.04 -- bash /mnt/c/mDev/Polaris/@prime/tools/setup/verify-p1-eventlog-disk-full.sh
    if ($LASTEXITCODE -ne 0) { throw 'Disk-full eventlog regression failed' }
} finally {
    if ($mounted) {
        & wsl -d Ubuntu-24.04 -u root -- umount $linuxDirectory
        if ($LASTEXITCODE -ne 0) { throw 'Disposable test filesystem could not be unmounted' }
    }
}
