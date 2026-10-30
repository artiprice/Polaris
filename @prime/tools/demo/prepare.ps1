# Materialize the demo workspaces of @prime/DEMO.md (P10-5) from the synthetic fixtures.
#
#   powershell -File @prime/tools/demo/prepare.ps1                 # into $env:TEMP\polaris-demo
#   powershell -File @prime/tools/demo/prepare.ps1 -Root D:\demo   # elsewhere
#
# Fixture files end in `.fixture` so this repository never looks like a CUDA or service project;
# they are copied without the suffix. The Rust workspace gets one deliberate bug (is_prime treats
# squares of primes as prime) for the model to find and fix. Existing demo folders are replaced.
param([string]$Root = (Join-Path $env:TEMP "polaris-demo"))
$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$fixtures = Join-Path $repo "tests\fixtures"

function Materialize([string]$From, [string]$To) {
    if (Test-Path $To) { Remove-Item -Recurse -Force $To }
    $source = Join-Path $fixtures $From
    Get-ChildItem -Recurse -File $source | ForEach-Object {
        $relative = $_.FullName.Substring($source.Length + 1)
        if ($relative.EndsWith(".fixture")) { $relative = $relative.Substring(0, $relative.Length - 8) }
        $target = Join-Path $To $relative
        New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
        Copy-Item $_.FullName $target
    }
}

New-Item -ItemType Directory -Force $Root | Out-Null
$rust = Join-Path $Root "rust-bug"
Materialize "ci\rust-lib" $rust
$lib = Join-Path $rust "src\lib.rs"
$text = [System.IO.File]::ReadAllText($lib)
$buggy = $text.Replace("take_while(|d| d * d <= n)", "take_while(|d| d * d < n)")
if ($buggy -eq $text) { throw "the Rust fixture changed; update the injected bug in prepare.ps1" }
[System.IO.File]::WriteAllText($lib, $buggy)

Materialize "nvidia\cuda-vector-add" (Join-Path $Root "cuda")
Materialize "hack\seeded-service" (Join-Path $Root "hack")

Write-Output "Demo workspaces in ${Root}:"
Write-Output "  rust-bug  ci/rust-lib with is_prime off by one (4, 9 and 25 count as prime)"
Write-Output "  cuda      nvidia/cuda-vector-add (one advisor finding per kernel)"
Write-Output "  hack      hack/seeded-service (seeded races, blocking calls, a divergent barrier)"
