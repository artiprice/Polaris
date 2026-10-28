$ErrorActionPreference = 'Stop'
$wrapper = Join-Path $PSScriptRoot 'build_verified.ps1'
$repo = Split-Path -Parent $PSScriptRoot
$originalThreads = $env:RUST_TEST_THREADS
$originalLocation = Get-Location
function Assert($condition, $message) {
    if (-not $condition) { throw $message }
}
try {
    # Shadow only Python: exercise the real wrapper without rebuilding for each path.
    function global:python {
        Assert ($env:RUST_TEST_THREADS -eq '1') 'Build must run serially'
        Assert ((Get-Location).Path -eq $repo) 'Build must run at repository root'
        Assert ($args.Count -eq 3) 'Unexpected build argument count'
        Assert ($args[0] -eq (Join-Path $repo 'scripts/build_static.py')) 'Wrong build script'
        Assert ($args[1] -eq '--offline' -and $args[2] -eq '--test') 'Test gate bypassed'
        if ($global:PolarisWrapperSimulateThrow) { throw 'simulated launch failure' }
        $global:LASTEXITCODE = $global:PolarisWrapperSimulatedExit
    }
    Set-Location $PSScriptRoot
    foreach ($previous in @($null, '7')) {
        foreach ($code in @(0, 37)) {
            $env:RUST_TEST_THREADS = $previous
            $global:PolarisWrapperSimulatedExit = $code
            & $wrapper
            Assert ($LASTEXITCODE -eq $code) 'Native exit code was not propagated'
            Assert ($env:RUST_TEST_THREADS -eq $previous) 'Thread environment was not restored'
            Assert ((Get-Location).Path -eq $PSScriptRoot) 'Working directory was not restored'
        }
    }
    $global:PolarisWrapperSimulateThrow = $true
    $caught = $false
    try { & $wrapper } catch { $caught = $_.Exception.Message -eq 'simulated launch failure' }
    Assert $caught 'Launch failure was swallowed'
    Assert ($env:RUST_TEST_THREADS -eq '7') 'Environment was not restored after exception'
    Assert ((Get-Location).Path -eq $PSScriptRoot) 'Location was not restored after exception'
    Write-Output 'PASS: wrapper arguments, serial tests, success/failure exit codes, environment/location restoration, launch exception'
}
finally {
    Remove-Item -LiteralPath Function:\python
    Remove-Variable -Name PolarisWrapperSimulatedExit, PolarisWrapperSimulateThrow -Scope Global -ErrorAction SilentlyContinue
    $env:RUST_TEST_THREADS = $originalThreads
    Set-Location $originalLocation
}
