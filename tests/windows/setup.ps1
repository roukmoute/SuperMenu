<#
.SYNOPSIS
Tests the actual per-user installer with unique registry/shortcut names and owned directories.
.EXAMPLE
pwsh -NoProfile -File tests/windows/setup.ps1 -Executable target/release/supermenu.exe -NsisDirectory tools/nsis -Capture
#>
#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$NsisDirectory = '',
    [string]$Fixture = (Join-Path $PSScriptRoot '../fixtures/launcher.json'),
    [string]$ExpectedVersion = '',
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [switch]$Capture
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'support.ps1')
if (-not [Environment]::Is64BitProcess) { throw 'The setup tests require 64-bit PowerShell.' }
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$exe = [IO.Path]::GetFullPath($Executable)
$fixturePath = [IO.Path]::GetFullPath($Fixture)
if (-not [IO.File]::Exists($exe) -or -not [IO.File]::Exists($fixturePath)) { throw 'The payload and fixture must exist.' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $Report) { $Report = Join-Path $OutputDirectory 'setup.json' }
if (-not $ExpectedVersion) {
    $manifest = [IO.File]::ReadAllText((Join-Path $repository 'Cargo.toml'))
    $ExpectedVersion = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $ExpectedVersion) { throw 'The expected package version is missing.' }
}

$workspace = New-TestWorkspace
$namespace = 'SuperMenuSetupFixture-' + [Guid]::NewGuid().ToString('D')
$namespaceToken = $namespace.Substring('SuperMenuSetupFixture-'.Length)
$installDirectory = Join-Path $workspace.Root 'Application Super Menu · café'
$installedExe = Join-Path $installDirectory 'supermenu.exe'
$uninstaller = Join-Path $installDirectory 'uninstall.exe'
$dataDirectory = Join-Path $workspace.Root 'Données utilisateur'
$appDataDirectory = Join-Path $dataDirectory 'SuperMenu'
$arpKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $namespace
$foreignArpKey = $arpKey + '-Foreign'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$foreignRunValue = $namespace + '-Foreign'
$startMenuRoot = [Environment]::GetFolderPath('Programs')
$desktopRoot = [Environment]::GetFolderPath('DesktopDirectory')
if (-not $startMenuRoot -or -not $desktopRoot) { throw 'The current-user shortcut folders are unavailable.' }
$menuDirectory = Join-Path $startMenuRoot $namespace
$menuLink = Join-Path $menuDirectory 'Super Menu.lnk'
$desktopLink = Join-Path $desktopRoot ($namespace + '.lnk')
$menuMarker = Join-Path $menuDirectory '.supermenu-test-workspace'
$foreignMenuFile = Join-Path $menuDirectory 'À conserver.txt'
$foreignInstallFile = Join-Path $installDirectory 'À conserver.txt'
$checks = [Collections.Generic.List[object]]::new()
$cases = [Collections.Generic.List[object]]::new()
$invocations = [Collections.Generic.List[object]]::new()
$contexts = [Collections.Generic.List[object]]::new()
$context = [pscustomobject]@{ Checks = $checks }
$shell = $null
$unknown = $null
$failed = $false
$failure = ''
$ownsNamespace = $false

function Get-RunValue([string]$Name) {
    if (-not (Test-Path -LiteralPath $runKey)) { return $null }
    $key = Get-Item -LiteralPath $runKey
    try { return $key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
    finally { $key.Dispose() }
}

function Get-FileDigest([string]$Path) {
    if (-not [IO.File]::Exists($Path)) { return '' }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Get-RegistrySnapshot([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return 'absent' }
    $key = Get-Item -LiteralPath $Path
    try {
        $values = [ordered]@{}
        foreach ($name in @($key.GetValueNames() | Sort-Object)) {
            $values[$name] = [ordered]@{
                kind = [string]$key.GetValueKind($name)
                value = $key.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            }
        }
        return ($values | ConvertTo-Json -Depth 8 -Compress)
    } finally { $key.Dispose() }
}

function Get-ProtectedSnapshot {
    # Compare only the production application's registration and named links;
    # the test never writes these locations or reads other software's data.
    return ([ordered]@{
        arp = Get-RegistrySnapshot 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\SuperMenu'
        startup = Get-RunValue 'SuperMenu'
        desktop = Get-FileDigest (Join-Path $desktopRoot 'Super Menu.lnk')
        menu = Get-FileDigest (Join-Path $startMenuRoot 'Super Menu\Super Menu.lnk')
    } | ConvertTo-Json -Depth 8 -Compress)
}

function Invoke-OwnedProgram([string]$File, [string[]]$Arguments, [string]$Name, [int]$TimeoutSeconds = 30, [string]$RawArguments = '') {
    $start = [Diagnostics.ProcessStartInfo]::new($File)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['LOCALAPPDATA'] = $dataDirectory
    if ($RawArguments) { $start.Arguments = $RawArguments }
    else { foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) } }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process = [Diagnostics.Process]::Start($start)
    try {
        # Drain concurrently so diagnostic output cannot fill a pipe and stall.
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) { throw ($Name + ' timed out.') }
        $timer.Stop()
        $result = [pscustomobject]@{
            name = $Name; exit_code = $process.ExitCode; elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 1)
            stdout = $stdout.GetAwaiter().GetResult(); stderr = $stderr.GetAwaiter().GetResult()
        }
        $invocations.Add($result)
        return $result
    } finally {
        if (-not $process.HasExited) { $process.Kill(); [void]$process.WaitForExit(5000) }
        $process.Dispose()
    }
}

function Start-IsolatedLauncher([string]$Path) {
    $launcherWorkspace = New-TestWorkspace
    $launcher = $null
    try {
        $snapshot = Join-Path $launcherWorkspace.Root 'snapshot.json'
        $start = [Diagnostics.ProcessStartInfo]::new($Path)
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $start.Environment['LOCALAPPDATA'] = $dataDirectory
        foreach ($argument in @('--smoke-test', '--smoke-fixture', $fixturePath, '--smoke-theme', 'dark',
            '--smoke-timeout-ms', '300000', '--report', $snapshot)) { $start.ArgumentList.Add($argument) }
        $process = [Diagnostics.Process]::Start($start)
        $launcher = [pscustomobject]@{
            Process = $process; Pid = [uint32]$process.Id
            Api = [SuperMenuTests.OwnedWindowApi]::new([uint32]$process.Id)
            Main = [IntPtr]::Zero; Workspace = $launcherWorkspace; ReportPath = $snapshot
            Deadline = [DateTime]::UtcNow.AddSeconds(300)
            Checks = [Collections.Generic.List[object]]::new(); Theme = 'dark'
        }
        $contexts.Add($launcher)
        Wait-TestCondition -Context $launcher -Message 'the isolated installed launcher window' -Condition {
            $launcher.Main = $launcher.Api.FindWindow('SuperMenuWindow')
            return $launcher.Main -ne [IntPtr]::Zero
        }
        $native = Read-TestSnapshot -Context $launcher
        Assert-Test -Context $context -Condition ([bool]$native.fixture_mode -and $native.fixture_activation_attempts -eq 0) -Name 'installed launcher uses the inert fixture index'
        return $launcher
    } catch {
        if ($null -eq $launcher) { Remove-TestWorkspace -Workspace $launcherWorkspace }
        throw
    }
}

function Invoke-Case([string]$Name, [scriptblock]$Action) {
    $case = [ordered]@{ case = $Name; passed = $false }
    try { & $Action; $case.passed = $true }
    finally { $cases.Add($case) }
}

function Assert-DataPreserved {
    foreach ($path in $dataDigests.Keys) {
        Assert-Test -Context $context -Condition ((Get-FileDigest $path) -ceq $dataDigests[$path]) -Name ('user data preserved: ' + [IO.Path]::GetFileName($path))
    }
}

function Assert-Installed {
    Assert-Test -Context $context -Condition ([IO.File]::Exists($installedExe)) -Name 'installer creates the single application payload'
    Assert-Test -Context $context -Condition ((Get-FileDigest $installedExe) -ceq $payloadDigest) -Name 'installed payload is byte-identical to the reviewed executable'
    Assert-Test -Context $context -Condition ([IO.File]::Exists($uninstaller)) -Name 'installer creates its uninstaller'
    foreach ($name in @('LICENSE', 'THIRD_PARTY_NOTICES.md', 'brand.ico')) {
        Assert-Test -Context $context -Condition ([IO.File]::Exists((Join-Path $installDirectory $name))) -Name ('installer includes ' + $name)
    }
    Assert-Test -Context $context -Condition (Test-Path -LiteralPath $arpKey) -Name 'installer registers only the fixture ARP entry'
    $arp = Get-ItemProperty -LiteralPath $arpKey
    Assert-Test -Context $context -Condition ($arp.DisplayName.Contains('Super Menu')) -Name 'ARP entry identifies Super Menu'
    Assert-Test -Context $context -Condition ($arp.DisplayVersion -ceq $ExpectedVersion) -Name 'ARP entry has the package version'
    Assert-Test -Context $context -Condition ($arp.InstallLocation.TrimEnd('\') -ieq $installDirectory.TrimEnd('\')) -Name 'ARP entry retains the Unicode installation path'
    Assert-Test -Context $context -Condition ($arp.UninstallString -ceq ('"' + $uninstaller + '"')) -Name 'ARP uninstall command quotes the owned executable'
    Assert-Test -Context $context -Condition ($arp.QuietUninstallString -ceq ('"' + $uninstaller + '" /S')) -Name 'ARP quiet uninstall command is available'
    Assert-Test -Context $context -Condition ($arp.NoModify -eq 1 -and $arp.NoRepair -eq 1) -Name 'ARP does not offer unsupported modification or repair'
    Assert-Test -Context $context -Condition ([IO.File]::Exists($menuLink)) -Name 'installer creates its current-user Start Menu shortcut'
    $link = $shell.CreateShortcut($menuLink)
    Assert-Test -Context $context -Condition ($link.TargetPath -ieq $installedExe -and $link.WorkingDirectory -ieq $installDirectory) -Name 'Start Menu shortcut targets the installed application'
}

try {
    Assert-Test -Context $context -Condition (-not (Test-Path -LiteralPath $arpKey) -and -not (Test-Path -LiteralPath $foreignArpKey)) -Name 'unique fixture registry entries are initially absent'
    Assert-Test -Context $context -Condition ($null -eq (Get-RunValue $namespace) -and $null -eq (Get-RunValue $foreignRunValue)) -Name 'unique fixture startup values are initially absent'
    Assert-Test -Context $context -Condition (-not [IO.Directory]::Exists($menuDirectory) -and -not [IO.File]::Exists($desktopLink)) -Name 'unique fixture shortcuts are initially absent'
    $ownsNamespace = $true
    $protected = Get-ProtectedSnapshot
    $payloadDigest = Get-FileDigest $exe
    $shell = New-Object -ComObject WScript.Shell
    [void][IO.Directory]::CreateDirectory($appDataDirectory)
    $dataDigests = [ordered]@{}
    foreach ($file in @('settings.json', 'index.json', 'usage.json', 'profile-preserved.txt')) {
        $path = Join-Path $appDataDirectory $file
        [IO.File]::WriteAllText($path, ('{"setup_fixture":"' + $namespaceToken + '","file":"' + $file + '"}'), [Text.UTF8Encoding]::new($false))
        $dataDigests[$path] = Get-FileDigest $path
    }
    New-Item -Path $foreignArpKey -Force | Out-Null
    New-ItemProperty -LiteralPath $foreignArpKey -Name 'DisplayName' -Value $namespaceToken -PropertyType String -Force | Out-Null
    if (-not (Test-Path -LiteralPath $runKey)) { New-Item -Path $runKey | Out-Null }
    New-ItemProperty -LiteralPath $runKey -Name $foreignRunValue -Value $namespaceToken -PropertyType String -Force | Out-Null
    $buildArguments = @{
        Executable = $exe; OutputDirectory = Join-Path $workspace.Root 'setup-build'
        Version = $ExpectedVersion; TestNamespace = $namespaceToken; TestInstallDirectory = $installDirectory
    }
    if ($NsisDirectory) { $buildArguments.NsisDirectory = [IO.Path]::GetFullPath($NsisDirectory) }
    $buildLog = Join-Path $OutputDirectory 'setup-build.log'
    [void][IO.Directory]::CreateDirectory($OutputDirectory)
    & (Join-Path $repository 'scripts/build-setup.ps1') @buildArguments *> $buildLog
    if (-not $?) { throw 'The isolated setup build failed.' }
    $setups = @(Get-ChildItem -LiteralPath $buildArguments.OutputDirectory -File -Filter ('*' + $namespaceToken + '*.exe'))
    Assert-Test -Context $context -Condition ($setups.Count -eq 1) -Name 'the builder creates one uniquely namespaced setup executable'
    $setup = $setups[0].FullName

    if ($Capture) {
        if (-not ('SuperMenuTests.SetupCapture' -as [type])) {
            Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
namespace SuperMenuTests {
    public static class SetupCapture {
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr hwnd, int command);
        public static void ShowWithoutActivation(IntPtr hwnd, uint expectedPid) {
            uint actualPid;
            GetWindowThreadProcessId(hwnd, out actualPid);
            if (hwnd == IntPtr.Zero || actualPid != expectedPid || !IsWindow(hwnd))
                throw new InvalidOperationException("The installer HWND is not owned by this capture process.");
            if (!ShowWindowAsync(hwnd, 4)) // SW_SHOWNOACTIVATE, never request focus.
                throw new InvalidOperationException("The owned installer could not be shown for capture.");
        }
    }
}
"@
        }
        # The welcome page is read-only. Terminate only this owned fixture process
        # after capture, without advancing the installer or touching desktop input.
        $captureStart = [Diagnostics.ProcessStartInfo]::new($setup)
        $captureStart.UseShellExecute = $false
        $captureStart.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $captureProcess = [Diagnostics.Process]::Start($captureStart)
        try {
            $captureContext = [pscustomobject]@{
                Process = $captureProcess; Pid = [uint32]$captureProcess.Id
                Api = [SuperMenuTests.OwnedWindowApi]::new([uint32]$captureProcess.Id)
                Deadline = [DateTime]::UtcNow.AddSeconds(15); Main = [IntPtr]::Zero
            }
            Wait-TestCondition -Context $captureContext -Message 'owned installer welcome page' -Condition {
                $captureContext.Main = $captureContext.Api.FindWindow('#32770')
                return $captureContext.Main -ne [IntPtr]::Zero -and
                    @($captureContext.Api.GetControls($captureContext.Main) | Where-Object { $_.Text -match 'Install(?:er)? Super Menu' }).Count -gt 0
            }
            # Hidden startup is honored by NSIS on Welcome. Show only the owned
            # dialog without activation, once the localized text is initialized.
            [SuperMenuTests.SetupCapture]::ShowWithoutActivation($captureContext.Main, $captureContext.Pid)
            Wait-TestCondition -Context $captureContext -Message 'owned installer is ready to paint without activation' -Condition {
                return $captureContext.Api.IsVisible($captureContext.Main)
            }
            Start-Sleep -Milliseconds 100
            [void](Save-TestWindow -Context $captureContext -Window $captureContext.Main -Path (Join-Path $OutputDirectory 'setup-welcome.png'))
            [void]$captureContext.Api.Send($captureContext.Main, 0x0111, 1, 0) # Owned Next button.
            Wait-TestCondition -Context $captureContext -Message 'owned installer optional shortcut page' -Condition {
                try {
                    return @($captureContext.Api.GetControls($captureContext.Main) | Where-Object {
                        $_.Class -ieq 'Button' -and ($_.Text -match 'Windows' -or $_.Text -match 'desktop|Bureau')
                    }).Count -eq 2
                } catch [InvalidOperationException] {
                    # NSIS can destroy an enumerated child while switching pages.
                    # Retry with fresh PID-checked handles within the same deadline.
                    return $false
                }
            }
            [void](Save-TestWindow -Context $captureContext -Window $captureContext.Main -Path (Join-Path $OutputDirectory 'setup-options.png'))
        } finally {
            if (-not $captureProcess.HasExited) { $captureProcess.Kill(); [void]$captureProcess.WaitForExit(5000) }
            $captureProcess.Dispose()
        }
    }

    Invoke-Case 'first silent installation leaves optional startup and desktop links disabled' {
        $result = Invoke-OwnedProgram $setup @('/S') 'first silent install'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'first silent install succeeds without confirmation'
        Assert-Installed
        Assert-Test -Context $context -Condition ($null -eq (Get-RunValue $namespace)) -Name 'startup is disabled by default'
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($desktopLink)) -Name 'desktop shortcut is disabled by default'
        Assert-DataPreserved
        [IO.File]::WriteAllText($foreignInstallFile, $namespaceToken)
        [IO.File]::WriteAllText($foreignMenuFile, $namespaceToken)
        [IO.File]::WriteAllText($menuMarker, $namespaceToken)
    }

    Invoke-Case 'explicit optional startup and desktop shortcuts can be enabled' {
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=1', '/DESKTOP=1') 'enable optional shortcuts'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'explicit optional shortcuts install silently'
        Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq ('"' + $installedExe + '" --background')) -Name 'startup command quotes the installed payload and requests background mode'
        Assert-Test -Context $context -Condition ([IO.File]::Exists($desktopLink)) -Name 'explicit desktop shortcut is created'
        $desktopShortcut = $shell.CreateShortcut($desktopLink)
        Assert-Test -Context $context -Condition ($desktopShortcut.TargetPath -ieq $installedExe) -Name 'desktop shortcut targets the installed payload'
    }

    Invoke-Case 'shutdown selects an exact image path among simultaneous launcher windows' {
        $script:installedLauncher = Start-IsolatedLauncher $installedExe
        $foreignDirectory = Join-Path $workspace.Root 'Autre launcher · café'
        [void][IO.Directory]::CreateDirectory($foreignDirectory)
        $foreignExecutable = Join-Path $foreignDirectory 'supermenu.exe'
        [IO.File]::Copy($exe, $foreignExecutable)
        $script:foreignLauncher = Start-IsolatedLauncher $foreignExecutable
        $absentDirectory = Join-Path $workspace.Root 'Absent launcher'
        [void][IO.Directory]::CreateDirectory($absentDirectory)
        $absent = Invoke-OwnedProgram $exe @('--shutdown-for-install', (Join-Path $absentDirectory 'supermenu.exe')) 'shutdown absent exact path'
        Assert-Test -Context $context -Condition ($absent.exit_code -eq 0) -Name 'absent expected executable requires no shutdown'
        Assert-Test -Context $context -Condition (-not $installedLauncher.Process.HasExited -and -not $foreignLauncher.Process.HasExited) -Name 'absent-path shutdown leaves both owned launcher windows running'
        $result = Invoke-OwnedProgram $exe @('--shutdown-for-install', $installedExe) 'shutdown exact installed launcher'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'exact installed launcher shuts down successfully'
        Assert-Test -Context $context -Condition ($installedLauncher.Process.WaitForExit(5000) -and $installedLauncher.Process.ExitCode -eq 0) -Name 'requested launcher exits gracefully'
        Assert-Test -Context $context -Condition (-not $foreignLauncher.Process.HasExited) -Name 'other executable path remains running despite matching window class'
    }

    Invoke-Case 'upgrade closes only the installed launcher and preserves options and user files' {
        $script:installedLauncher = Start-IsolatedLauncher $installedExe
        $result = Invoke-OwnedProgram $setup @('/S') 'upgrade running fixture'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'upgrade succeeds while the installed fixture is running'
        Assert-Test -Context $context -Condition ($installedLauncher.Process.WaitForExit(5000) -and $installedLauncher.Process.ExitCode -eq 0) -Name 'upgrade closes the installed launcher gracefully before copying its executable'
        Assert-Test -Context $context -Condition (-not $foreignLauncher.Process.HasExited) -Name 'upgrade keeps the other launcher path running'
        Assert-Installed
        Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq ('"' + $installedExe + '" --background')) -Name 'upgrade preserves existing startup choice'
        Assert-Test -Context $context -Condition ([IO.File]::Exists($desktopLink)) -Name 'upgrade preserves existing desktop shortcut choice'
        $desktopShortcut = $shell.CreateShortcut($desktopLink)
        Assert-Test -Context $context -Condition ($desktopShortcut.TargetPath -ieq $installedExe) -Name 'upgrade keeps the desktop shortcut pointing at the installed payload'
        Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($foreignInstallFile) -ceq $namespaceToken) -Name 'upgrade preserves foreign installation-directory files'
        Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($foreignMenuFile) -ceq $namespaceToken) -Name 'upgrade preserves foreign Start Menu files'
        Assert-DataPreserved
    }

    Invoke-Case 'shutdown rejects an exact-path non-launcher without force-killing it' {
        $unknownDirectory = Join-Path $workspace.Root 'Processus inconnu'
        [void][IO.Directory]::CreateDirectory($unknownDirectory)
        $unknownExecutable = Join-Path $unknownDirectory 'supermenu.exe'
        [IO.File]::Copy((Join-Path $env:SystemRoot 'System32\cmd.exe'), $unknownExecutable)
        $unknownStart = [Diagnostics.ProcessStartInfo]::new($unknownExecutable)
        $unknownStart.UseShellExecute = $false
        $unknownStart.CreateNoWindow = $true
        $unknownStart.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $unknownStart.RedirectStandardInput = $true
        $unknownStart.RedirectStandardOutput = $true
        $unknownStart.ArgumentList.Add('/d')
        $unknownStart.ArgumentList.Add('/q')
        $unknownStart.ArgumentList.Add('/c')
        $unknownStart.ArgumentList.Add('pause')
        $script:unknown = [Diagnostics.Process]::Start($unknownStart)
        Start-Sleep -Milliseconds 75
        Assert-Test -Context $context -Condition (-not $unknown.HasExited) -Name 'the owned non-launcher remains blocked on its private stdin'
        $result = Invoke-OwnedProgram $exe @('--shutdown-for-install', $unknownExecutable) 'reject owned non-launcher'
        Assert-Test -Context $context -Condition ($result.exit_code -ne 0) -Name 'an exact-path process without a launcher window is rejected'
        Assert-Test -Context $context -Condition (-not $unknown.HasExited) -Name 'shutdown never force-kills an unknown process'
        Assert-Test -Context $context -Condition (-not $foreignLauncher.Process.HasExited) -Name 'rejected unknown process cannot affect the other launcher'
        $unknown.StandardInput.WriteLine('x')
        if (-not $unknown.WaitForExit(5000)) { throw 'The owned cmd fixture did not finish after its private input.' }
    }

    Invoke-Case 'explicit optional settings can be disabled on upgrade' {
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=0', '/DESKTOP=0') 'disable optional shortcuts'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'optional shortcuts can be disabled silently'
        Assert-Test -Context $context -Condition ($null -eq (Get-RunValue $namespace)) -Name 'explicit startup disable removes only the fixture Run value'
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($desktopLink)) -Name 'explicit desktop disable removes only the fixture shortcut'
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=1', '/DESKTOP=1') 'restore options for uninstall coverage'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'optional shortcuts are restored for uninstall coverage'
    }

    Invoke-Case 'a locked executable aborts upgrade without corrupting the existing install' {
        $registeredBefore = Get-RegistrySnapshot $arpKey
        $desktopBefore = Get-FileDigest $desktopLink
        $lock = [IO.FileStream]::new($installedExe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            $result = Invoke-OwnedProgram $setup @('/S') 'reject locked executable upgrade'
            Assert-Test -Context $context -Condition ($result.exit_code -ne 0) -Name 'locked executable produces a nonzero upgrade result'
            Assert-Test -Context $context -Condition ((Get-FileDigest $installedExe) -ceq $payloadDigest) -Name 'failed upgrade keeps the unchanged old executable'
            Assert-Test -Context $context -Condition ((Get-RegistrySnapshot $arpKey) -ceq $registeredBefore) -Name 'failed upgrade keeps the previous ARP registration intact'
            Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq ('"' + $installedExe + '" --background')) -Name 'failed upgrade keeps the existing startup choice'
            Assert-Test -Context $context -Condition ((Get-FileDigest $desktopLink) -ceq $desktopBefore) -Name 'failed upgrade leaves the existing desktop shortcut intact'
            Assert-Test -Context $context -Condition ([IO.File]::Exists($uninstaller)) -Name 'failed upgrade retains a usable uninstaller'
            Assert-DataPreserved
        } finally { $lock.Dispose() }
    }

    Invoke-Case 'desktop disable preserves a shortcut repointed by its owner' {
        $script:foreignShortcutTarget = Join-Path $env:SystemRoot 'System32\cmd.exe'
        $shortcut = $shell.CreateShortcut($desktopLink)
        $shortcut.TargetPath = $foreignShortcutTarget
        $shortcut.Arguments = '/d /c exit'
        $shortcut.Save()
        $script:foreignDesktopDigest = Get-FileDigest $desktopLink
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=1', '/DESKTOP=0') 'preserve repointed desktop link'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'desktop disable with a repointed link succeeds'
        Assert-Test -Context $context -Condition ((Get-FileDigest $desktopLink) -ceq $foreignDesktopDigest) -Name 'desktop disable preserves a repointed shortcut byte-for-byte'
        $shortcut = $shell.CreateShortcut($menuLink)
        $shortcut.TargetPath = $foreignShortcutTarget
        $shortcut.Arguments = '/d /c exit'
        $shortcut.Save()
        $script:foreignMenuDigest = Get-FileDigest $menuLink
        Assert-Test -Context $context -Condition ($shortcut.TargetPath -ieq $foreignShortcutTarget) -Name 'Start Menu shortcut is deliberately repointed without executing it'
    }

    Invoke-Case 'a locked executable aborts uninstall and preserves usable registration' {
        $lock = [IO.FileStream]::new($installedExe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            # Only this failure case bypasses NSIS relocation to observe the actual
            # uninstaller exit code. It aborts before deleting its own executable.
            # NSIS consumes the entire final _?= path and requires it unquoted.
            $result = Invoke-OwnedProgram -File $uninstaller -Arguments @() -Name 'reject locked executable uninstall' -RawArguments ('/S _?=' + $installDirectory)
            Assert-Test -Context $context -Condition ($result.exit_code -ne 0) -Name 'locked program file produces a nonzero uninstall result'
            Assert-Test -Context $context -Condition ([IO.File]::Exists($installedExe) -and (Get-FileDigest $installedExe) -ceq $payloadDigest) -Name 'failed uninstall keeps the unchanged executable'
            Assert-Test -Context $context -Condition ([IO.File]::Exists($uninstaller)) -Name 'failed uninstall keeps its retryable uninstaller'
            Assert-Test -Context $context -Condition (Test-Path -LiteralPath $arpKey) -Name 'failed uninstall keeps ARP registration for retry'
            Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq ('"' + $installedExe + '" --background')) -Name 'failed uninstall keeps the existing startup entry'
            Assert-Test -Context $context -Condition ((Get-FileDigest $desktopLink) -ceq $foreignDesktopDigest -and (Get-FileDigest $menuLink) -ceq $foreignMenuDigest) -Name 'failed uninstall keeps repointed shortcuts'
            Assert-DataPreserved
        } finally { $lock.Dispose() }
    }

    Invoke-Case 'successful uninstall preserves repointed shortcuts and unrelated data' {
        $foreignCommand = '"' + $foreignShortcutTarget + '" /d /c exit'
        Set-ItemProperty -LiteralPath $runKey -Name $namespace -Value $foreignCommand
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=1', '/DESKTOP=1') 'upgrade with repointed shortcuts and startup value'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'upgrade with repointed integration points succeeds'
        Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq $foreignCommand) -Name 'explicit startup enable preserves a foreign command under the exact fixture name'
        Assert-Test -Context $context -Condition ((Get-FileDigest $desktopLink) -ceq $foreignDesktopDigest -and (Get-FileDigest $menuLink) -ceq $foreignMenuDigest) -Name 'upgrade preserves repointed desktop and Start Menu links'
        $script:installedLauncher = Start-IsolatedLauncher $installedExe
        $result = Invoke-OwnedProgram $uninstaller @('/S') 'uninstall with repointed shortcuts'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'uninstall can retry successfully after the file lock is released'
        Assert-Test -Context $context -Condition ($installedLauncher.Process.WaitForExit(5000) -and $installedLauncher.Process.ExitCode -eq 0) -Name 'retry uninstall closes the installed launcher gracefully'
        $uninstallDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while (([IO.File]::Exists($installedExe) -or [IO.File]::Exists($uninstaller) -or
            (Test-Path -LiteralPath $arpKey)) -and [DateTime]::UtcNow -lt $uninstallDeadline) {
            Start-Sleep -Milliseconds 25
        }
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($installedExe) -and -not [IO.File]::Exists($uninstaller)) -Name 'successful retry removes the program and its uninstaller'
        Assert-Test -Context $context -Condition (-not (Test-Path -LiteralPath $arpKey)) -Name 'successful retry removes ARP registration'
        Assert-Test -Context $context -Condition ((Get-RunValue $namespace) -ceq $foreignCommand) -Name 'successful retry preserves a repointed startup value under the exact fixture name'
        Assert-Test -Context $context -Condition ((Get-FileDigest $desktopLink) -ceq $foreignDesktopDigest) -Name 'uninstaller preserves the repointed desktop link byte-for-byte'
        Assert-Test -Context $context -Condition ((Get-FileDigest $menuLink) -ceq $foreignMenuDigest) -Name 'uninstaller preserves the repointed Start Menu link byte-for-byte'
        Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($foreignInstallFile) -ceq $namespaceToken) -Name 'retry preserves foreign installation-directory files'
        Assert-DataPreserved
        # These exact links were created and repointed by this fixture. Remove
        # them here so the final case verifies removal of ordinary owned links.
        Remove-Item -LiteralPath $desktopLink -Force
        Remove-Item -LiteralPath $menuLink -Force
        Remove-ItemProperty -LiteralPath $runKey -Name $namespace
        $result = Invoke-OwnedProgram $setup @('/S', '/STARTUP=1', '/DESKTOP=1') 'reinstall owned links for final uninstall'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'fixture can reinstall ordinary owned shortcuts after complete uninstall'
        Assert-Installed
    }

    Invoke-Case 'real silent uninstall removes only owned program files and registration' {
        $script:installedLauncher = Start-IsolatedLauncher $installedExe
        # NSIS relocates its uninstaller so it can remove the installed copy.
        # Wait for the owned registration/files to settle after that bootstrap.
        $result = Invoke-OwnedProgram $uninstaller @('/S') 'silent uninstall running fixture'
        Assert-Test -Context $context -Condition ($result.exit_code -eq 0) -Name 'uninstaller succeeds without repeated confirmations'
        Assert-Test -Context $context -Condition ($installedLauncher.Process.WaitForExit(5000) -and $installedLauncher.Process.ExitCode -eq 0) -Name 'uninstaller closes the installed launcher gracefully'
        $uninstallDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while (([IO.File]::Exists($installedExe) -or [IO.File]::Exists($uninstaller) -or
            (Test-Path -LiteralPath $arpKey)) -and [DateTime]::UtcNow -lt $uninstallDeadline) {
            Start-Sleep -Milliseconds 25
        }
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($installedExe)) -Name 'uninstaller removes the installed application executable'
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($uninstaller)) -Name 'uninstaller removes its installed copy after relocation'
        Assert-Test -Context $context -Condition (-not (Test-Path -LiteralPath $arpKey)) -Name 'uninstaller removes only its fixture ARP key'
        Assert-Test -Context $context -Condition ($null -eq (Get-RunValue $namespace)) -Name 'uninstaller removes only its fixture startup value'
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($menuLink) -and -not [IO.File]::Exists($desktopLink)) -Name 'uninstaller removes its Start Menu and desktop shortcuts'
        Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($foreignInstallFile) -ceq $namespaceToken) -Name 'uninstaller preserves foreign files in the installation directory'
        Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($foreignMenuFile) -ceq $namespaceToken) -Name 'uninstaller preserves foreign files in the Start Menu directory'
        Assert-Test -Context $context -Condition (Test-Path -LiteralPath $foreignArpKey) -Name 'uninstaller preserves a neighboring foreign fixture ARP entry'
        Assert-Test -Context $context -Condition ((Get-RunValue $foreignRunValue) -ceq $namespaceToken) -Name 'uninstaller preserves a neighboring foreign fixture startup value'
        Assert-Test -Context $context -Condition (-not $foreignLauncher.Process.HasExited) -Name 'uninstaller leaves the launcher at a different executable path running'
        Assert-DataPreserved
        Assert-Test -Context $context -Condition ((Get-ProtectedSnapshot) -ceq $protected) -Name 'the production application registration and shortcuts are unchanged'
    }
} catch {
    $failed = $true
    $failure = $_.Exception.Message
} finally {
    foreach ($launcher in $contexts) {
        try { Stop-TestLauncher -Context $launcher }
        catch { $failed = $true; $failure = 'Owned launcher cleanup failed: ' + $_.Exception.Message }
    }
    if ($null -ne $unknown) {
        if (-not $unknown.HasExited) { $unknown.Kill(); [void]$unknown.WaitForExit(5000) }
        $unknown.Dispose()
    }
    try {
        # Validate every exact registry/shortcut target before removing fixtures.
        if ($namespace -cnotmatch '^SuperMenuSetupFixture-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$') { throw 'Invalid cleanup namespace.' }
        if ($ownsNamespace) {
            foreach ($key in @($arpKey, $foreignArpKey)) {
                if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
            }
            foreach ($value in @($namespace, $foreignRunValue)) {
                if ($null -ne (Get-RunValue $value)) { Remove-ItemProperty -LiteralPath $runKey -Name $value }
            }
            if ([IO.File]::Exists($desktopLink)) { Remove-Item -LiteralPath $desktopLink -Force }
            if ([IO.Directory]::Exists($menuDirectory)) {
                $resolvedMenu = [IO.Path]::GetFullPath($menuDirectory)
                if ([IO.Path]::GetDirectoryName($resolvedMenu) -ine [IO.Path]::GetFullPath($startMenuRoot).TrimEnd('\') -or
                    [IO.Path]::GetFileName($resolvedMenu) -cne $namespace) { throw 'Refusing unsafe Start Menu fixture cleanup.' }
                # The unique absent-at-start namespace belongs exclusively to this test.
                Remove-Item -LiteralPath $resolvedMenu -Recurse -Force
            }
        }
        Remove-TestWorkspace -Workspace $workspace
    } catch { $failed = $true; $failure = 'Setup fixture cleanup failed: ' + $_.Exception.Message }
    if ($null -ne $shell) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) }
    $result = [ordered]@{
        suite = 'native per-user setup'; passed = -not $failed; version = $ExpectedVersion
        namespace = $namespace; cases = $cases.ToArray(); checks = $checks.ToArray(); invocations = $invocations.ToArray()
    }
    if ($failed) { $result.error = $failure }
    [void](Write-TestReport -Result $result -Path $Report)
    Write-Output ('Setup: ' + $checks.Count + ' assertions, ' + $cases.Count + ' cases, report: ' + [IO.Path]::GetFullPath($Report))
}
if ($failed) { throw ('Native setup tests failed: ' + $failure) }
