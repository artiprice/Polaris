# =============================================================================
# Polaris - per-member Windows setup (run from the repo root in a normal, non-admin
# PowerShell; winget may raise its own UAC prompts for individual installers).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File @prime\tools\setup\setup-windows.ps1 [-Check]
#
# -Check            verify only: print every component and its state, install nothing
# -SkipOllama       skip Ollama + the nemotron-mini model (~2.7 GB download)
# -SkipWsl          skip the WSL2 Ubuntu-24.04 distro, its user and setup-wsl.sh
# -RegisterKeepalive  register a per-user logon task that keeps the WSL2 distro (and the
#                   OpenShell gateway inside it) from idling out (pitfall W14)
#
# Idempotent: each step checks first. Exit code 0 when every required component is
# present afterwards, 1 otherwise. See @prime/PHASES.md section 0.1 "Team member setup".
# ASCII only (Windows PowerShell 5.1 reads BOM-less files as ANSI).
# =============================================================================
param(
    [switch]$Check,
    [switch]$SkipOllama,
    [switch]$SkipWsl,
    [switch]$RegisterKeepalive
)

$ErrorActionPreference = 'Stop'
$Distro = 'Ubuntu-24.04'
$WslUser = 'dev'
$RustVersion = '1.99.0'
$KeepaliveTask = 'Polaris WSL keepalive'
# Versions verified on the reference machine (PHASES_STATUS P0-ENV-3).
$CargoTools = [ordered]@{
    'cargo-nextest' = '0.9.148'
    'cargo-mutants' = '27.1.0'
    'cargo-leptos'  = '0.3.11'
    'cargo-deny'    = '0.20.2'
    'cargo-vet'     = '0.10.2'
    'cargo-audit'   = '0.22.2'
}
$BinstallVersion = '1.26.0'
$UvVersion = '0.13.3'
$OllamaModel = 'nemotron-mini'
# winget id -> command that proves it is usable
$WingetPackages = [ordered]@{
    'Git.Git'           = 'git'
    'Rustlang.Rustup'   = 'rustup'
    'Python.Python.3.14' = 'python'
    'Kitware.CMake'     = 'cmake'
    'NASM.NASM'         = 'nasm'
    'Ninja-build.Ninja' = 'ninja'
    'Google.Protobuf'   = 'protoc'
}

$script:Missing = New-Object System.Collections.Generic.List[string]

function Write-Step([string]$Text) { Write-Host ''; Write-Host "== $Text" }
function Write-State([string]$Name, [bool]$Ok, [string]$Detail) {
    $mark = if ($Ok) { 'ok     ' } else { 'MISSING' }
    Write-Host ('  [{0}] {1,-22} {2}' -f $mark, $Name, $Detail)
    if (-not $Ok) { $script:Missing.Add($Name) }
}
function Test-Command([string]$Name) { return [bool](Get-Command $Name -ErrorAction SilentlyContinue) }
function Update-SessionPath {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machine;$user;$env:USERPROFILE\.cargo\bin"
}
function First-Line([scriptblock]$Block) {
    # Stdout only, under a local 'Continue': in Windows PowerShell 5.1 a native command's
    # redirected stderr (e.g. rustup's "info:" line) raises an exception under 'Stop'.
    $ErrorActionPreference = 'Continue'
    try {
        $line = (& $Block 2>$null) | ForEach-Object { "$_".Trim() } | Where-Object { $_ } | Select-Object -First 1
        return [string]$line
    } catch { return '' }
}
function Invoke-Native([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

if (-not (Test-Path -LiteralPath (Join-Path (Get-Location) '@prime\tools\workspace.toml'))) {
    Write-Host 'run from the Polaris repository root'
    exit 2
}
Update-SessionPath

# ---------------------------------------------------------------------------
Write-Step '1/8 Windows prerequisites'
$build = [Environment]::OSVersion.Version.Build
Write-State 'windows-build' ($build -ge 19045) "build $build (WSL2 needs >= 19045)"
$longPaths = 0
try { $longPaths = Get-ItemPropertyValue 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -Name LongPathsEnabled } catch { $longPaths = 0 }
Write-State 'LongPathsEnabled' ($longPaths -eq 1) $(if ($longPaths -eq 1) { '1' } else { 'admin: Set-ItemProperty HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem LongPathsEnabled 1' })
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msvc = ''
if (Test-Path -LiteralPath $vswhere) {
    $msvc = First-Line { & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationVersion }
}
if (-not $msvc -and -not $Check) {
    winget install --id Microsoft.VisualStudio.BuildTools --exact --accept-source-agreements --accept-package-agreements --override '--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    if (Test-Path -LiteralPath $vswhere) {
        $msvc = First-Line { & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationVersion }
    }
}
Write-State 'msvc-build-tools' ([bool]$msvc) $(if ($msvc) { "VS $msvc (VC tools x64)" } else { 'winget Microsoft.VisualStudio.BuildTools + VCTools workload' })

# ---------------------------------------------------------------------------
Write-Step '2/8 winget packages'
foreach ($id in $WingetPackages.Keys) {
    $cmd = $WingetPackages[$id]
    if (-not (Test-Command $cmd) -and -not $Check) {
        winget install --id $id --exact --silent --accept-source-agreements --accept-package-agreements
        Update-SessionPath
    }
    $ok = Test-Command $cmd
    $ver = ''
    if ($ok) {
        switch ($cmd) {
            'nasm'   { $ver = First-Line { nasm -v } }
            'protoc' { $ver = First-Line { protoc --version } }
            default  { $ver = First-Line { & $cmd --version } }
        }
    }
    Write-State $cmd $ok $(if ($ok) { $ver } else { "winget install --id $id --exact" })
}

# ---------------------------------------------------------------------------
Write-Step "3/8 Rust $RustVersion"
if ((Test-Command 'rustup') -and -not $Check) {
    Invoke-Native 'rustup' @('toolchain', 'install', $RustVersion, '--profile', 'minimal', '-c', 'rustfmt', '-c', 'clippy')
}
$rustc = ''
if (Test-Command 'rustup') { $rustc = First-Line { rustup run $RustVersion rustc --version } }
Write-State "rust-$RustVersion" ($rustc -like "rustc $RustVersion*") $(if ($rustc) { $rustc } else { "rustup toolchain install $RustVersion" })

# ---------------------------------------------------------------------------
Write-Step '4/8 cargo tools'
if (-not (Test-Command 'cargo-binstall') -and -not $Check -and (Test-Command 'cargo')) {
    # Built from crates.io source (checksum-verified by cargo) instead of piping a remote script.
    Invoke-Native 'cargo' @('install', '--locked', 'cargo-binstall', '--version', $BinstallVersion)
}
Write-State 'cargo-binstall' (Test-Command 'cargo-binstall') (First-Line { cargo binstall -V })
foreach ($tool in $CargoTools.Keys) {
    $want = $CargoTools[$tool]
    $sub = $tool.Substring(6)
    $have = ''
    if (Test-Command $tool) { $have = First-Line { cargo $sub --version } }
    if (($have -notmatch [regex]::Escape($want)) -and -not $Check -and (Test-Command 'cargo-binstall')) {
        Invoke-Native 'cargo' @('binstall', '--no-confirm', '--locked', "$tool@$want")
        $have = First-Line { cargo $sub --version }
    }
    Write-State $tool ($have -match [regex]::Escape($want)) $(if ($have) { "$have (want $want)" } else { "cargo binstall $tool@$want" })
}

# ---------------------------------------------------------------------------
Write-Step "5/8 uv $UvVersion (python -m uv)"
$uv = ''
if (Test-Command 'python') { $uv = First-Line { python -m uv --version } }
if (($uv -notmatch [regex]::Escape($UvVersion)) -and -not $Check -and (Test-Command 'python')) {
    Invoke-Native 'python' @('-m', 'pip', 'install', '--user', '--quiet', "uv==$UvVersion")
    $uv = First-Line { python -m uv --version }
}
Write-State 'uv' ($uv -match [regex]::Escape($UvVersion)) $(if ($uv -like 'uv *') { $uv } else { "python -m pip install --user uv==$UvVersion" })

# ---------------------------------------------------------------------------
Write-Step '6/8 repository-local git config'
if (-not $Check) { Invoke-Native 'git' @('config', '--local', 'core.longpaths', 'true') }
$lp = First-Line { git config --local --get core.longpaths }
Write-State 'git core.longpaths' ($lp -eq 'true') $(if ($lp) { $lp } else { 'git config --local core.longpaths true' })

# ---------------------------------------------------------------------------
Write-Step "7/8 Ollama + $OllamaModel (P3 local endpoint)"
if ($SkipOllama) {
    Write-Host '  skipped (-SkipOllama)'
} else {
    if (-not (Test-Command 'ollama') -and -not $Check) {
        winget install --id Ollama.Ollama --exact --silent --accept-source-agreements --accept-package-agreements
        Update-SessionPath
    }
    $ollama = Test-Command 'ollama'
    Write-State 'ollama' $ollama $(if ($ollama) { First-Line { ollama --version } } else { 'winget install --id Ollama.Ollama --exact' })
    if ($ollama) {
        $models = ((ollama list 2>$null) | Out-String)
        if (($models -notmatch [regex]::Escape($OllamaModel)) -and -not $Check) {
            Invoke-Native 'ollama' @('pull', $OllamaModel)
            $models = ((ollama list 2>$null) | Out-String)
        }
        $hasModel = $models -match [regex]::Escape($OllamaModel)
        Write-State $OllamaModel $hasModel $(if ($hasModel) { 'pulled' } else { "ollama pull $OllamaModel" })
    }
}

# ---------------------------------------------------------------------------
Write-Step "8/8 WSL2 $Distro + OpenShell (via setup-wsl.sh)"
if ($SkipWsl) {
    Write-Host '  skipped (-SkipWsl)'
} else {
    $distros = ((wsl.exe --list --quiet) | Out-String) -replace "`0", ''
    $installed = $distros -match [regex]::Escape($Distro)
    if (-not $installed -and -not $Check) {
        Invoke-Native 'wsl.exe' @('--install', '-d', $Distro, '--no-launch')
        Invoke-Native 'wsl.exe' @('-d', $Distro, '-u', 'root', '--', 'true')
        $distros = ((wsl.exe --list --quiet) | Out-String) -replace "`0", ''
        $installed = $distros -match [regex]::Escape($Distro)
    }
    Write-State $Distro $installed $(if ($installed) { 'installed' } else { "wsl --install -d $Distro --no-launch (may need a reboot the first time)" })
    if ($installed) {
        $hasUser = First-Line { wsl.exe -d $Distro -u root -- id -u $WslUser }
        if (($hasUser -notmatch '^\d+$') -and -not $Check) {
            # Locked password + passwordless sudo, so setup-wsl.sh runs unattended. Set a password
            # afterwards if your policy requires one: wsl -d Ubuntu-24.04 -u root passwd dev
            $bootstrap = "useradd -m -s /bin/bash -G sudo $WslUser && " +
                "echo '$WslUser ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/90-$WslUser && chmod 440 /etc/sudoers.d/90-$WslUser && " +
                "(getent group kvm >/dev/null && usermod -aG kvm $WslUser || true) && " +
                "printf '[boot]\nsystemd=true\n\n[user]\ndefault=$WslUser\n' > /etc/wsl.conf"
            Invoke-Native 'wsl.exe' @('-d', $Distro, '-u', 'root', '--', 'bash', '-c', $bootstrap)
            Invoke-Native 'wsl.exe' @('--terminate', $Distro)
            $hasUser = First-Line { wsl.exe -d $Distro -u root -- id -u $WslUser }
        }
        Write-State "wsl user $WslUser" ($hasUser -match '^\d+$') 'default user with sudo (see script comment)'
        # wsl.exe strips backslashes from arguments, so hand wslpath a forward-slash path.
        $repoWin = (Get-Location).Path -replace '\\', '/'
        $repoWsl = First-Line { wsl.exe -d $Distro -- wslpath -a $repoWin }
        if (-not $repoWsl.StartsWith('/')) { throw "wslpath could not convert '$repoWin'" }
        if (-not $Check -and ($hasUser -match '^\d+$')) {
            Invoke-Native 'wsl.exe' @('-d', $Distro, '--', 'bash', "$repoWsl/@prime/tools/setup/setup-wsl.sh")
        }
        $os = First-Line { wsl.exe -d $Distro -- bash -lc 'openshell --version' }
        Write-State 'openshell (wsl)' ($os -match '0\.1\.4') $(if ($os) { $os } else { 'run setup-wsl.sh' })
        $gw = First-Line { wsl.exe -d $Distro -- bash -lc 'systemctl --user is-active openshell-gateway' }
        Write-State 'openshell-gateway' ($gw -eq 'active') $gw
    }
    if ($RegisterKeepalive -and -not $Check) {
        $action = New-ScheduledTaskAction -Execute 'wsl.exe' -Argument "-d $Distro --exec sleep infinity"
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
        $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -Hidden
        Register-ScheduledTask -TaskName $KeepaliveTask -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
        Start-ScheduledTask -TaskName $KeepaliveTask
    }
    $task = Get-ScheduledTask -TaskName $KeepaliveTask -ErrorAction SilentlyContinue
    $taskDetail = if ($task) { "scheduled task: $($task.State)" } else { 'optional: re-run with -RegisterKeepalive, or keep a wsl session open' }
    Write-Host ('  [{0}] {1,-22} {2}' -f $(if ($task) { 'ok     ' } else { 'info   ' }), 'wsl keepalive', $taskDetail)
}

# ---------------------------------------------------------------------------
Write-Host ''
if ($script:Missing.Count -eq 0) {
    Write-Host 'RESULT: all required components present'
    exit 0
}
Write-Host ("RESULT: missing -> {0}" -f ($script:Missing -join ', '))
exit 1
