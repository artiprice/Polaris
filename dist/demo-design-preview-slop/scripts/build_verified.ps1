$ErrorActionPreference = 'Stop'
$previousThreads = $env:RUST_TEST_THREADS
$buildExit = 1
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $env:RUST_TEST_THREADS = '1'
    & python (Join-Path $PSScriptRoot 'build_static.py') --offline --test
    $buildExit = $LASTEXITCODE
}
finally {
    $env:RUST_TEST_THREADS = $previousThreads
    Pop-Location
}
exit $buildExit
