# Real, local-only acceptance against the fixed std-only fixture and explicitly supplied image.
# Each case owns a new state directory. Existing sessions/artifacts are never overwritten.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Image,
    [Parameter(Mandatory = $true)][string]$MtlsDirectory,
    [string]$Endpoint = 'https://localhost:17670',
    [string]$WslDistro = 'Ubuntu',
    [string]$Binary = '',
    [ValidateSet('positive', 'zero-tests', 'failing-command', 'minimum-tests')]
    [string[]]$Cases = @('positive', 'zero-tests', 'failing-command', 'minimum-tests')
)
# Failures are thrown explicitly; 'Stop' would also turn a refused command's stderr into a fatal
# NativeCommandError in Windows PowerShell 5.1.
$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$binary = if ($Binary) { $Binary } else { Join-Path $repository 'target/release/polaris.exe' }
$fixture = Join-Path $PSScriptRoot 'acceptance-fixture'
$policy = Join-Path $PSScriptRoot 'acceptance-policy.yaml'
if (!(Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Build polaris-cli first (cargo build --release -p polaris-cli).' }
if ($Image -notmatch '(^sha256:[0-9a-f]{64}$|@sha256:[0-9a-f]{64}$)') { throw 'An immutable image digest is required.' }
foreach ($name in @('ca.crt', 'tls.crt', 'tls.key')) {
    if (!(Test-Path -LiteralPath (Join-Path $MtlsDirectory $name) -PathType Leaf)) { throw "Missing mTLS file: $name" }
}
$outputRoot = Join-Path $repository ('@prime/tmp/p2/headless-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($outputRoot)
$encoding = [Text.UTF8Encoding]::new($false)
$ordinary = @('cargo', 'test', '--offline', '--locked', '--lib')
$independent = @('cargo', 'test', '--offline', '--locked', '--test', 'falsifier')
$results = @()

function Assert-Cleanup($Transcript) {
    if ($null -eq $Transcript.cleanup -or $Transcript.cleanup.deadline_exceeded -ne $false -or $Transcript.cleanup.unresolved_resources -ne 0) {
        throw 'Acceptance failed: unresolved resources or missing cleanup evidence.'
    }
}

foreach ($case in $Cases) {
    $caseRoot = Join-Path $outputRoot $case
    [void][IO.Directory]::CreateDirectory($caseRoot)
    $request = @{argv = $ordinary; independent_argv = $independent; timeout_secs = 120; min_tests = 3}
    switch ($case) {
        'zero-tests' { $request.argv = $ordinary + @('--', '__polaris_no_matching_test_9769__') }
        'failing-command' { $request.argv = @('cargo', 'test', '--offline', '--locked', '--package', 'polaris-package-does-not-exist') }
        'minimum-tests' { $request.min_tests = 4 }
        'positive' { }
    }
    $requestPath = Join-Path $caseRoot 'request.json'
    [IO.File]::WriteAllText($requestPath, ($request | ConvertTo-Json -Depth 8), $encoding)
    $stateRoot = Join-Path $caseRoot 'state'
    $gatewayArgs = @('sandbox', '--image', $Image, '--policy', $policy, '--mtls-dir', $MtlsDirectory, '--endpoint', $Endpoint, '--wsl-distro', $WslDistro)
    $reviewPath = Join-Path $caseRoot 'review.json'
    & $binary --state-dir $stateRoot --workspace $fixture @gatewayArgs review --request $requestPath 1> $reviewPath 2> (Join-Path $caseRoot 'review.stderr.txt')
    if ($LASTEXITCODE -ne 0) { throw "Review failed: $case" }
    $review = Get-Content -LiteralPath $reviewPath -Raw | ConvertFrom-Json
    Assert-Cleanup $review
    $alerts = @($review.events | Where-Object { $_.event.type -eq 'blast_radius_alert' })
    if ($alerts.Count -ne 1 -or $alerts[0].event.report -ne $review.review.digest) { throw 'Review lacks the exact durable blast alert.' }
    $executePath = Join-Path $caseRoot 'execute.json'
    & $binary --state-dir $stateRoot --workspace $fixture --session $review.session @gatewayArgs execute --review $review.review.digest --approve $review.review.digest --frame $review.frame 1> $executePath 2> (Join-Path $caseRoot 'execute.stderr.txt')
    $exitCode = $LASTEXITCODE
    $execute = Get-Content -LiteralPath $executePath -Raw | ConvertFrom-Json
    Assert-Cleanup $execute
    $passed = @($execute.events | Where-Object { $_.event.type -eq 'proof_of_done_passed' })
    $blocked = @($execute.events | Where-Object { $_.event.type -eq 'proof_of_done_blocked' })
    $runs = @($execute.events | Where-Object { $_.event.type -eq 'sandbox_execution_completed' })
    if ($runs.Count -ne 1) { throw "Expected one actual ordinary sandbox run: $case" }
    if ($case -eq 'positive') {
        if ($exitCode -ne 0 -or $execute.status -ne 'proof_passed' -or $passed.Count -ne 1 -or $blocked.Count -ne 0 -or $runs[0].event.exit_code -ne 0) {
            throw 'Positive acceptance failed.'
        }
    } elseif ($exitCode -eq 0 -or $execute.status -ne 'blocked' -or $passed.Count -ne 0 -or $blocked.Count -ne 1) {
        throw "Negative acceptance did not block: $case"
    }
    $results += [pscustomobject]@{case = $case; exit_code = $exitCode; status = $execute.status; sandbox_runs = $runs.Count; proof_passed = $passed.Count; proof_blocked = $blocked.Count; unresolved_resources = $execute.cleanup.unresolved_resources; artifacts = $caseRoot}
    # A consumed review may never execute twice, even after process/kernel restart.
    if ($case -eq 'positive') {
        $replayPath = Join-Path $caseRoot 'replay.json'
        & $binary --state-dir $stateRoot --workspace $fixture --session $review.session @gatewayArgs execute --review $review.review.digest --approve $review.review.digest --frame $review.frame 1> $replayPath 2> (Join-Path $caseRoot 'replay.stderr.txt')
        if ($LASTEXITCODE -eq 0) { throw 'Replayed approval unexpectedly succeeded.' }
        $replay = Get-Content -LiteralPath $replayPath -Raw | ConvertFrom-Json
        Assert-Cleanup $replay
        if ($replay.status -ne 'blocked') { throw 'Replay did not return blocked.' }
        $replayRuns = @($replay.events | Where-Object { $_.event.type -eq 'sandbox_execution_completed' })
        $replayPasses = @($replay.events | Where-Object { $_.event.type -eq 'proof_of_done_passed' })
        if ($replayRuns.Count -ne 1 -or $replayPasses.Count -ne 1) { throw 'Replay added another run/proof to the durable transcript.' }
    }
    $results[-1] | ConvertTo-Json -Compress
}
[IO.File]::WriteAllText((Join-Path $outputRoot 'summary.json'), ($results | ConvertTo-Json -Depth 8), $encoding)
exit 0
