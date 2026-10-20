# Builds the S0-9 spike and samples peak memory of all rustc/cargo/cc/cmake processes every 2 s.
# Writes build.log and build-metrics.txt next to this script.
param([int]$Jobs = 4)
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
$start = Get-Date
$build = Start-Process -FilePath cargo -ArgumentList @('build', '--release', '-j', "$Jobs") -NoNewWindow -PassThru `
    -RedirectStandardOutput build.out.log -RedirectStandardError build.log
$peakMB = 0
$peakFreeLowGB = [double]::MaxValue
while (-not $build.HasExited) {
    $procs = Get-Process -Name rustc, cargo, cc, cl, link, cmake, clang -ErrorAction SilentlyContinue
    $mb = [math]::Round((($procs | Measure-Object -Property WorkingSet64 -Sum).Sum) / 1MB)
    if ($mb -gt $peakMB) { $peakMB = $mb }
    $free = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1MB
    if ($free -lt $peakFreeLowGB) { $peakFreeLowGB = $free }
    Start-Sleep -Seconds 2
}
$build.WaitForExit()
$elapsed = [int]((Get-Date) - $start).TotalSeconds
$metrics = @(
    "exit_code=$($build.ExitCode)",
    "elapsed_s=$elapsed",
    "jobs=$Jobs",
    "peak_build_procs_working_set_mb=$peakMB",
    "lowest_free_physical_gb=$([math]::Round($peakFreeLowGB, 2))",
    "target_dir_gb=$([math]::Round(((Get-ChildItem target -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum) / 1GB, 2))",
    "protoc_on_path=$([bool](Get-Command protoc -ErrorAction SilentlyContinue))",
    "finished=$(Get-Date -Format s)"
)
$metrics | Set-Content -Encoding utf8 build-metrics.txt
$metrics
