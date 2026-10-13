#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [switch]$Capture
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'support.ps1')
if (-not [Environment]::Is64BitProcess) { throw 'The updater tests require 64-bit PowerShell.' }
$exe = [IO.Path]::GetFullPath($Executable)
if (-not [IO.File]::Exists($exe)) { throw 'The updater executable must exist.' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $Report) { $Report = Join-Path $OutputDirectory 'updater.json' }
$workspace = New-TestWorkspace
$checks = [Collections.Generic.List[object]]::new()
$cases = [Collections.Generic.List[object]]::new()
$ownedProcesses = [Collections.Generic.List[object]]::new()
$context = [pscustomobject]@{ Checks = $checks }
$failed = $false
$failure = ''

# Only these throwaway executables act as the old/new portable application. Their
# hidden window records the real helper's commit message; --background records
# the real restart. They never use Super Menu's global mutex, hotkey or indexing,
# so an interactive installation already running on the PC cannot be activated.
$fixtureSource = @'
using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;

class UpdateFixture {
    delegate IntPtr WindowProcedure(IntPtr window, uint message, IntPtr wp, IntPtr lp);
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] struct WindowClass {
        public uint size, style;
        public WindowProcedure procedure;
        public int extraClass, extraWindow;
        public IntPtr instance, icon, cursor, background;
        public string menu, className;
        public IntPtr smallIcon;
    }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct Message {
        public IntPtr hwnd; public uint message; public UIntPtr wp; public IntPtr lp;
        public uint time; public Point point; public uint privateValue;
    }
    [StructLayout(LayoutKind.Sequential)] struct FileTime { public uint low, high; }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern IntPtr GetModuleHandleW(string name);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern bool GetProcessTimes(IntPtr process, out FileTime created, out FileTime exited, out FileTime kernel, out FileTime user);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern ushort RegisterClassExW(ref WindowClass description);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint extra, string name, string title, uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr DefWindowProcW(IntPtr window, uint message, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll")] static extern void PostQuitMessage(int code);
    [DllImport("user32.dll")] static extern int GetMessageW(out Message message, IntPtr window, uint first, uint last);
    [DllImport("user32.dll")] static extern bool TranslateMessage(ref Message message);
    [DllImport("user32.dll")] static extern IntPtr DispatchMessageW(ref Message message);
    static readonly WindowProcedure procedure = ProcessMessage;
    static string root;
    static ulong request;
    static bool hold;

    static void WriteMarker(string name, string contents) {
        File.WriteAllText(Path.Combine(root, name), contents);
    }

    static IntPtr ProcessMessage(IntPtr window, uint message, IntPtr wp, IntPtr lp) {
        if (message == 0x800E) {
            ulong supplied = unchecked((ulong)wp.ToInt64());
            WriteMarker("commit.json", "{\"request\":" + supplied + "}");
            if (supplied == request && !hold) DestroyWindow(window);
            return IntPtr.Zero;
        }
        if (message == 0x800F) {
            WriteMarker("aborted.txt", wp.ToInt64().ToString());
            return IntPtr.Zero;
        }
        if (message == 0x0010) { DestroyWindow(window); return IntPtr.Zero; }
        if (message == 0x0002) { PostQuitMessage(0); return IntPtr.Zero; }
        return DefWindowProcW(window, message, wp, lp);
    }

    static int Main(string[] arguments) {
        root = Environment.GetEnvironmentVariable("SUPER_MENU_UPDATER_FIXTURE_ROOT");
        if (String.IsNullOrEmpty(root) || !Directory.Exists(root)) return 90;
        if (arguments.Length == 1 && arguments[0] == "--background") {
#if UPDATED
            const string generation = "updated";
#else
            const string generation = "original";
#endif
            WriteMarker("restart.json", "{\"generation\":\"" + generation + "\",\"background\":true,\"pid\":" + Process.GetCurrentProcess().Id + "}");
            return 0;
        }
        request = UInt64.Parse(Environment.GetEnvironmentVariable("SUPER_MENU_UPDATER_FIXTURE_REQUEST"));
        hold = Environment.GetEnvironmentVariable("SUPER_MENU_UPDATER_FIXTURE_HOLD") == "1";
        IntPtr instance = GetModuleHandleW(null);
        var description = new WindowClass { size = (uint)Marshal.SizeOf(typeof(WindowClass)), procedure = procedure, instance = instance, className = "SuperMenuWindow" };
        if (RegisterClassExW(ref description) == 0) return 91;
        IntPtr window = CreateWindowExW(0, description.className, "Super Menu updater fixture", 0x80000000, 0, 0, 32, 32, IntPtr.Zero, IntPtr.Zero, instance, IntPtr.Zero);
        if (window == IntPtr.Zero) return 92;
        FileTime created, exited, kernel, user;
        if (!GetProcessTimes(GetCurrentProcess(), out created, out exited, out kernel, out user)) return 93;
        ulong createdValue = ((ulong)created.high << 32) | created.low;
        WriteMarker("ready.json", "{\"pid\":" + Process.GetCurrentProcess().Id + ",\"hwnd\":" + window.ToInt64() + ",\"created\":" + createdValue + "}");
        Message message;
        while (GetMessageW(out message, IntPtr.Zero, 0, 0) > 0) {
            TranslateMessage(ref message); DispatchMessageW(ref message);
        }
        GC.KeepAlive(procedure);
        return 0;
    }
}
'@

function Get-Digest([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Test-JsonMarker([string]$Path) {
    # File creation can become visible before the fixture's small write finishes.
    # Poll a complete JSON document rather than accepting mere file existence.
    try { return $null -ne ([IO.File]::ReadAllText($Path) | ConvertFrom-Json -ErrorAction Stop) }
    catch { return $false }
}

function Start-OwnedProgram([string]$File, [string[]]$Arguments, [string]$CaseRoot, [string]$LocalData, [bool]$Hold = $false) {
    $start = [Diagnostics.ProcessStartInfo]::new($File)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['LOCALAPPDATA'] = $LocalData
    $start.Environment['SUPER_MENU_UPDATER_FIXTURE_ROOT'] = $CaseRoot
    $start.Environment['SUPER_MENU_UPDATER_FIXTURE_REQUEST'] = '1729'
    $start.Environment['SUPER_MENU_UPDATER_FIXTURE_HOLD'] = $(if ($Hold) { '1' } else { '0' })
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($start)
    $owned = [pscustomobject]@{
        Process = $process; Pid = [uint32]$process.Id
        Api = [SuperMenuTests.OwnedWindowApi]::new([uint32]$process.Id)
        Main = [IntPtr]::Zero; Deadline = [DateTime]::UtcNow.AddSeconds(45)
        Stdout = $process.StandardOutput.ReadToEndAsync(); Stderr = $process.StandardError.ReadToEndAsync()
    }
    $ownedProcesses.Add($owned)
    return $owned
}

function New-UpdateCase([string]$Name, [bool]$Hold = $false) {
    $caseRoot = Join-Path $workspace.Root $Name
    $targetDirectory = Join-Path $caseRoot 'portable with space É'
    $localData = Join-Path $caseRoot 'local data é'
    $dataDirectory = Join-Path $localData 'SuperMenu'
    [void][IO.Directory]::CreateDirectory($targetDirectory)
    [void][IO.Directory]::CreateDirectory($dataDirectory)
    $target = Join-Path $targetDirectory 'supermenu.exe'
    [IO.File]::Copy($originalFixture, $target)
    $dataDigests = [ordered]@{}
    foreach ($name in @('settings.json', 'index.json', 'usage.json', 'profile-preserved.txt')) {
        $path = Join-Path $dataDirectory $name
        [IO.File]::WriteAllText($path, ('{"language":"fr","auto_update":false,"fixture":"' + $workspace.Token + '"}'), [Text.UTF8Encoding]::new($false))
        $dataDigests[$path] = Get-Digest $path
    }
    $parent = Start-OwnedProgram -File $target -Arguments @() -CaseRoot $caseRoot -LocalData $localData -Hold $Hold
    $readyPath = Join-Path $caseRoot 'ready.json'
    try { Wait-TestCondition -Context $parent -Message 'owned updater fixture window' -Condition { return Test-JsonMarker $readyPath } }
    catch {
        if ($parent.Process.HasExited) {
            throw ('Updater fixture exited with code ' + $parent.Process.ExitCode + ': ' + $parent.Stdout.GetAwaiter().GetResult() + $parent.Stderr.GetAwaiter().GetResult())
        }
        throw
    }
    $ready = [IO.File]::ReadAllText($readyPath) | ConvertFrom-Json
    $parent.Main = [IntPtr][long]$ready.hwnd
    $parent.Api.AssertWindow($parent.Main)
    $stage = Join-Path $dataDirectory ('updates\stage.' + $parent.Pid + '.' + [DateTime]::UtcNow.Ticks)
    [void][IO.Directory]::CreateDirectory($stage)
    $helper = Join-Path $stage 'update-helper.exe'
    $payload = Join-Path $stage 'supermenu.exe'
    $jobPath = Join-Path $stage 'job.json'
    [IO.File]::Copy($exe, $helper)
    [IO.File]::Copy($updatedFixture, $payload)
    $job = [ordered]@{
        version = '999.0.0'; target = $target; original_sha256 = Get-Digest $target
        payload_sha256 = Get-Digest $payload; payload_bytes = [uint64]([IO.FileInfo]::new($payload).Length)
        installed = $false; parent_pid = $parent.Pid; parent_created = [uint64]$ready.created
        parent_hwnd = [uint64]$ready.hwnd; request = [uint64]1729
    }
    return [pscustomobject]@{
        Root = $caseRoot; LocalData = $localData; Data = $dataDirectory; Digests = $dataDigests
        Target = $target; Stage = $stage; Helper = $helper; Payload = $payload; JobPath = $jobPath
        Job = $job; Parent = $parent; OriginalDigest = $job.original_sha256; UpdatedDigest = $job.payload_sha256
    }
}

function Start-Update($Case, [string[]]$Arguments = @()) {
    [IO.File]::WriteAllText($Case.JobPath, ($Case.Job | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    if ($Arguments.Count -eq 0) { $Arguments = @('--apply-update', $Case.JobPath) }
    return Start-OwnedProgram $Case.Helper $Arguments $Case.Root $Case.LocalData
}

function Finish-Helper($Helper, [int]$ExpectedCode, [string]$Name, [int]$TimeoutSeconds = 15) {
    if (-not $Helper.Process.WaitForExit($TimeoutSeconds * 1000)) { throw ('Updater helper timed out: ' + $Name) }
    $result = [ordered]@{
        name = $Name; exit_code = $Helper.Process.ExitCode
        stdout = $Helper.Stdout.GetAwaiter().GetResult(); stderr = $Helper.Stderr.GetAwaiter().GetResult()
    }
    Assert-Test -Context $context -Condition ($Helper.Process.ExitCode -eq $ExpectedCode) -Name ($Name + ': helper returns the expected code')
    return $result
}

function Assert-DataUnchanged($Case, [string]$Name) {
    foreach ($path in $Case.Digests.Keys) {
        Assert-Test -Context $context -Condition ((Get-Digest $path) -ceq $Case.Digests[$path]) -Name ($Name + ': preserves ' + [IO.Path]::GetFileName($path))
    }
}

function Wait-FixtureRestart($Case) {
    $restartPath = Join-Path $Case.Root 'restart.json'
    $restartDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-JsonMarker $restartPath) -and [DateTime]::UtcNow -lt $restartDeadline) { Start-Sleep -Milliseconds 25 }
    Assert-Test -Context $context -Condition (Test-JsonMarker $restartPath) -Name ([IO.Path]::GetFileName($Case.Root) + ': application is really restarted')
    $restart = [IO.File]::ReadAllText($restartPath) | ConvertFrom-Json
    # A marker is written immediately before the fixture exits. Wait for the
    # proven target image if it is still running, so cleanup cannot race its CLR.
    $process = $null
    try {
        try { $process = [Diagnostics.Process]::GetProcessById([int]$restart.pid) }
        catch [ArgumentException] { return $restart }
        if (-not $process.HasExited -and $process.MainModule.FileName -ieq $Case.Target) {
            if (-not $process.WaitForExit(5000)) { throw 'The restarted fixture did not finish.' }
        }
    } finally { if ($null -ne $process) { $process.Dispose() } }
    return $restart
}

function Assert-Rejected($Case, $Helper, [string]$Name, [int]$ExpectedCode = 1) {
    $result = Finish-Helper $Helper $ExpectedCode $Name
    Assert-Test -Context $context -Condition (-not $Case.Parent.Process.HasExited) -Name ($Name + ': parent remains running')
    Assert-Test -Context $context -Condition (-not [IO.File]::Exists((Join-Path $Case.Root 'commit.json'))) -Name ($Name + ': no commit is requested')
    Assert-Test -Context $context -Condition ((Get-Digest $Case.Target) -ceq $Case.OriginalDigest) -Name ($Name + ': target remains byte-identical')
    Assert-Test -Context $context -Condition (-not [IO.File]::Exists((Join-Path $Case.Root 'restart.json'))) -Name ($Name + ': no application restart is attempted')
    Assert-DataUnchanged $Case $Name
    $cases.Add($result)
}

try {
    $compiler = Join-Path $env:SystemRoot 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    if (-not [IO.File]::Exists($compiler)) { throw 'The built-in .NET Framework x64 fixture compiler is unavailable.' }
    $source = Join-Path $workspace.Root 'update-fixture.cs'
    $originalFixture = Join-Path $workspace.Root 'original-fixture.exe'
    $updatedFixture = Join-Path $workspace.Root 'updated-fixture.exe'
    [IO.File]::WriteAllText($source, $fixtureSource, [Text.UTF8Encoding]::new($false))
    foreach ($build in @(@($originalFixture, '/define:ORIGINAL'), @($updatedFixture, '/define:UPDATED'))) {
        $compile = Start-OwnedProgram $compiler @('/nologo', '/target:winexe', '/platform:x64', ('/out:' + $build[0]), $build[1], $source) $workspace.Root $workspace.Root
        [void](Finish-Helper $compile 0 ('compile ' + [IO.Path]::GetFileName($build[0])))
    }
    Assert-Test -Context $context -Condition ((Get-Digest $originalFixture) -cne (Get-Digest $updatedFixture)) -Name 'old and new portable fixtures have distinct payloads'

    $success = New-UpdateCase 'successful portable update' $true
    $helper = Start-Update $success
    try { Wait-TestCondition -Context $success.Parent -Message 'validated updater commit handshake' -Condition { return Test-JsonMarker (Join-Path $success.Root 'commit.json') } }
    catch {
        $diagnosticPath = Join-Path $success.Data 'updates\result.txt'
        $diagnostic = $(if ([IO.File]::Exists($diagnosticPath)) { [IO.File]::ReadAllText($diagnosticPath) } else { 'No application-result diagnostic was written.' })
        if ($helper.Process.HasExited) {
            throw ('Updater exited before its commit, code ' + $helper.Process.ExitCode + ': ' + $diagnostic + ' ' + $helper.Stdout.GetAwaiter().GetResult() + $helper.Stderr.GetAwaiter().GetResult())
        }
        throw ($_.Exception.Message + ' ' + $diagnostic)
    }
    $commit = [IO.File]::ReadAllText((Join-Path $success.Root 'commit.json')) | ConvertFrom-Json
    Assert-Test -Context $context -Condition ($commit.request -eq $success.Job.request) -Name 'commit preserves the pending update request token'
    Assert-Test -Context $context -Condition (-not $helper.Process.HasExited -and -not $success.Parent.Process.HasExited) -Name 'helper waits for the entire original process after requesting commit'
    Assert-Test -Context $context -Condition ((Get-Digest $success.Target) -ceq $success.OriginalDigest) -Name 'target is not replaced while the parent remains running'
    Assert-Test -Context $context -Condition (-not [IO.File]::Exists((Join-Path $success.Root 'restart.json'))) -Name 'replacement is not restarted before the original process exits'
    $success.Parent.Api.Post($success.Parent.Main, 0x0010, 0, 0)
    Assert-Test -Context $context -Condition ($success.Parent.Process.WaitForExit(5000) -and $success.Parent.Process.ExitCode -eq 0) -Name 'the owned parent closes gracefully without termination'
    $successResult = Finish-Helper $helper 0 'successful portable update'
    Assert-Test -Context $context -Condition ((Get-Digest $success.Target) -ceq $success.UpdatedDigest) -Name 'portable target receives the verified new executable atomically'
    $restart = Wait-FixtureRestart $success
    Assert-Test -Context $context -Condition ($restart.generation -ceq 'updated' -and [bool]$restart.background) -Name 'restart uses the new payload with --background'
    Assert-DataUnchanged $success 'successful portable update'
    $resultPath = Join-Path $success.Data 'updates\result.txt'
    Assert-Test -Context $context -Condition ([IO.File]::Exists($resultPath) -and [IO.File]::ReadAllText($resultPath).Contains('999.0.0')) -Name 'successful update retains a local version result'
    $cases.Add($successResult)

    $foreignWindow = New-UpdateCase 'foreign window fixture'
    foreach ($invalid in @('digest mismatch', 'original digest mismatch', 'size mismatch', 'wrong process creation', 'wrong parent image', 'wrong window owner', 'installed mode mismatch', 'downgrade', 'unknown job field')) {
        $case = New-UpdateCase $invalid
        switch ($invalid) {
            'digest mismatch' { $case.Job.payload_sha256 = '0' * 64 }
            'original digest mismatch' { $case.Job.original_sha256 = '0' * 64 }
            'size mismatch' { $case.Job.payload_bytes++ }
            'wrong process creation' { $case.Job.parent_created++ }
            'wrong parent image' { $case.Job.parent_pid = [uint32]$PID; $case.Job.parent_created = [uint64]([Diagnostics.Process]::GetCurrentProcess().StartTime.ToUniversalTime().ToFileTimeUtc()) }
            'wrong window owner' { $case.Job.parent_hwnd = [uint64]$foreignWindow.Parent.Main.ToInt64() }
            'installed mode mismatch' { $case.Job.installed = $true; [IO.File]::Move($case.Payload, (Join-Path $case.Stage 'SuperMenu-Setup-x64.exe')) }
            'downgrade' { $case.Job.version = '0.0.0' }
            'unknown job field' { $case.Job['unexpected'] = 'reject me' }
        }
        Assert-Rejected $case (Start-Update $case) $invalid
        Assert-Test -Context $context -Condition (-not $foreignWindow.Parent.Process.HasExited) -Name ($invalid + ': the foreign window owner remains untouched')
    }

    $badImage = New-UpdateCase 'non executable payload'
    [IO.File]::WriteAllText($badImage.Payload, ('This is not a Windows executable. ' * 8))
    $badImage.Job.payload_bytes = [uint64]([IO.FileInfo]::new($badImage.Payload).Length)
    $badImage.Job.payload_sha256 = Get-Digest $badImage.Payload
    Assert-Rejected $badImage (Start-Update $badImage) 'non executable payload'

    $wrongArchitecture = New-UpdateCase 'x86 payload rejection'
    $image = [IO.File]::ReadAllBytes($wrongArchitecture.Payload)
    $peOffset = [BitConverter]::ToUInt32($image, 0x3C)
    $image[$peOffset + 4] = 0x4C
    $image[$peOffset + 5] = 0x01
    [IO.File]::WriteAllBytes($wrongArchitecture.Payload, $image)
    $wrongArchitecture.Job.payload_sha256 = Get-Digest $wrongArchitecture.Payload
    Assert-Rejected $wrongArchitecture (Start-Update $wrongArchitecture) 'x86 payload rejection'

    $foreignJob = New-UpdateCase 'job outside helper stage'
    $foreignPath = Join-Path $foreignJob.Root 'job.json'
    [IO.File]::WriteAllText($foreignPath, ($foreignJob.Job | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Assert-Rejected $foreignJob (Start-Update $foreignJob @('--apply-update', $foreignPath)) 'job outside helper stage'

    $wrongHelper = New-UpdateCase 'wrong helper executable name'
    [IO.File]::Move($wrongHelper.Helper, (Join-Path $wrongHelper.Stage 'foreign-helper.exe'))
    $wrongHelper.Helper = Join-Path $wrongHelper.Stage 'foreign-helper.exe'
    Assert-Rejected $wrongHelper (Start-Update $wrongHelper) 'wrong helper executable name'

    $extraArgument = New-UpdateCase 'extra helper argument'
    Assert-Rejected $extraArgument (Start-Update $extraArgument @('--apply-update', $extraArgument.JobPath, '--background')) 'extra helper argument' 2

    $declined = New-UpdateCase 'parent declines commit' $true
    $declinedHelper = Start-Update $declined
    Wait-TestCondition -Context $declined.Parent -Message 'refused updater commit handshake' -Condition { return Test-JsonMarker (Join-Path $declined.Root 'commit.json') }
    $declinedResult = Finish-Helper $declinedHelper 3 'parent declines commit' 40
    Assert-Test -Context $context -Condition (-not $declined.Parent.Process.HasExited) -Name 'a refused commit never terminates the original process'
    Assert-Test -Context $context -Condition ((Get-Digest $declined.Target) -ceq $declined.OriginalDigest) -Name 'a refused commit never replaces the original executable'
    Assert-Test -Context $context -Condition (-not [IO.File]::Exists((Join-Path $declined.Root 'restart.json'))) -Name 'a refused commit never starts another application instance'
    $abortPath = Join-Path $declined.Root 'aborted.txt'
    Wait-TestCondition -Context $declined.Parent -Message 'bounded commit refusal notification' -Condition {
        try { return [IO.File]::ReadAllText($abortPath) -ceq [string]$declined.Job.request }
        catch { return $false }
    }
    Assert-Test -Context $context -Condition ([IO.File]::ReadAllText($abortPath) -ceq [string]$declined.Job.request) -Name 'commit refusal returns the same request token to the still-running parent'
    Assert-DataUnchanged $declined 'parent declines commit'
    $cases.Add($declinedResult)

    $locked = New-UpdateCase 'locked portable target'
    $lock = [IO.FileStream]::new($locked.Target, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $lockedHelper = Start-Update $locked
        $lockedResult = Finish-Helper $lockedHelper 1 'locked portable target'
        Assert-Test -Context $context -Condition ($locked.Parent.Process.WaitForExit(5000) -and $locked.Parent.Process.ExitCode -eq 0) -Name 'locked-target failure follows an orderly parent exit'
        Assert-Test -Context $context -Condition ((Get-Digest $locked.Target) -ceq $locked.OriginalDigest) -Name 'failed replacement retains the original portable binary'
        $restart = Wait-FixtureRestart $locked
        Assert-Test -Context $context -Condition ($restart.generation -ceq 'original' -and [bool]$restart.background) -Name 'failure restarts the original payload in the background'
        Assert-DataUnchanged $locked 'locked portable target'
        $cases.Add($lockedResult)
    } finally { $lock.Dispose() }
} catch {
    $failed = $true
    $failure = $_.Exception.Message
} finally {
    foreach ($owned in $ownedProcesses) {
        try {
            if (-not $owned.Process.HasExited) {
                $window = $owned.Api.FindWindow('SuperMenuWindow')
                if ($window -ne [IntPtr]::Zero) { $owned.Api.Post($window, 0x0010, 0, 0) }
                if (-not $owned.Process.WaitForExit(3000)) {
                    # This handle belongs only to a process started by this suite.
                    $owned.Process.Kill(); [void]$owned.Process.WaitForExit(5000)
                }
            }
            $owned.Process.Dispose()
        } catch { $failed = $true; $failure = 'Owned updater process cleanup failed: ' + $_.Exception.Message }
    }
    try { Remove-TestWorkspace -Workspace $workspace }
    catch { $failed = $true; $failure = 'Updater test cleanup failed: ' + $_.Exception.Message }
    $result = [ordered]@{ suite = 'native updater'; passed = -not $failed; cases = $cases.ToArray(); checks = $checks.ToArray() }
    if ($failed) { $result.error = $failure }
    [void](Write-TestReport -Result $result -Path $Report)
    Write-Output ('Updater: ' + $checks.Count + ' assertions, report: ' + [IO.Path]::GetFullPath($Report))
}
if ($failed) { throw ('Native updater tests failed: ' + $failure) }
