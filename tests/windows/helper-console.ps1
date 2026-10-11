#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [switch]$Capture
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'support.ps1')
$exe = [IO.Path]::GetFullPath($Executable)
if (-not [IO.File]::Exists($exe)) { throw 'The executable must exist.' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $Report) { $Report = Join-Path $OutputDirectory 'helper-console.json' }
$workspace = New-TestWorkspace
$checks = [Collections.Generic.List[object]]::new()
$cases = [Collections.Generic.List[object]]::new()
$context = [pscustomobject]@{ Checks = $checks }
$failed = $false
$failure = ''

# A separate, hidden, detached PowerShell process reads only the console belonging
# to a helper created by this suite. Attaching never modifies the console mode or
# injects keyboard input. The test runner and interactive desktop are untouched.
$consoleProbeSource = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;

namespace SuperMenuTests {
    public sealed class ConsoleSnapshot {
        public uint InputMode { get; set; }
        public string Text { get; set; }
    }
    public static class ConsoleProbe {
        [StructLayout(LayoutKind.Sequential)] private struct Coord { public short X, Y; }
        [StructLayout(LayoutKind.Sequential)] private struct SmallRect { public short L, T, R, B; }
        [StructLayout(LayoutKind.Sequential)] private struct BufferInfo {
            public Coord Size, Cursor; public ushort Attributes; public SmallRect Window; public Coord MaximumSize;
        }
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool AttachConsole(uint pid);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool FreeConsole();
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr CreateFileW(
            string name, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetConsoleMode(IntPtr handle, out uint mode);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetConsoleScreenBufferInfo(IntPtr handle, out BufferInfo info);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool ReadConsoleOutputCharacterW(
            IntPtr handle, [Out, MarshalAs(UnmanagedType.LPArray, ArraySubType=UnmanagedType.U2)] char[] text,
            uint length, Coord origin, out uint read);
        [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);

        public static ConsoleSnapshot Read(uint ownedPid) {
            // PowerShell can initialize a console despite CreateNoWindow. Detach
            // this isolated probe before attaching to the owned helper; the runner
            // and any other process keep their own console attachments.
            FreeConsole();
            // A failed attachment is retried only while the helper opens its console.
            bool attached = false;
            for (int attempt = 0; attempt < 80 && !attached; attempt++) {
                attached = AttachConsole(ownedPid);
                if (!attached) Thread.Sleep(10);
            }
            if (!attached) throw new Win32Exception(Marshal.GetLastWin32Error(), "Cannot attach to the owned helper console.");
            IntPtr input = new IntPtr(-1), output = new IntPtr(-1);
            try {
                input = CreateFileW("CONIN$", 0x80000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
                output = CreateFileW("CONOUT$", 0x80000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
                if (input == new IntPtr(-1) || output == new IntPtr(-1))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                uint mode = 0;
                string contents = "";
                for (int attempt = 0; attempt < 80; attempt++) {
                    BufferInfo info;
                    if (!GetConsoleMode(input, out mode) || !GetConsoleScreenBufferInfo(output, out info))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    // Read the bounded visible buffer without consuming stdin.
                    uint length = (uint)Math.Min(8192, Math.Max(0, (int)info.Size.X * Math.Max(1, (int)info.Window.B + 1)));
                    var text = new char[(int)length];
                    uint read;
                    if (!ReadConsoleOutputCharacterW(output, text, length, new Coord(), out read))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    if (read > length) throw new InvalidOperationException("The console returned more cells than requested.");
                    // This API returns a cell count, without requiring a NUL terminator.
                    contents = new string(text, 0, (int)read).TrimEnd();
                    // Waiting for the heading avoids observing mode between
                    // AllocConsole and the helper's explicit input setup.
                    if (contents.Contains("Google Chrome")) break;
                    Thread.Sleep(10);
                }
                return new ConsoleSnapshot { InputMode = mode, Text = contents };
            } finally {
                if (input != new IntPtr(-1)) CloseHandle(input);
                if (output != new IntPtr(-1)) CloseHandle(output);
                FreeConsole();
            }
        }
    }
}
'@

try {
    $probeAssembly = Join-Path $workspace.Root 'SuperMenuConsoleProbe.dll'
    Add-Type -TypeDefinition $consoleProbeSource -OutputAssembly $probeAssembly

    function Start-OwnedHelper([string]$Arguments, [string]$CaseRoot) {
        [void][IO.Directory]::CreateDirectory($CaseRoot)
        $start = [Diagnostics.ProcessStartInfo]::new($exe)
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $start.Arguments = $Arguments
        $start.Environment['LOCALAPPDATA'] = $CaseRoot
        return [Diagnostics.Process]::Start($start)
    }

    function Start-OwnedConsoleProbe([string]$CaseRoot) {
        [void][IO.Directory]::CreateDirectory($CaseRoot)
        $probePath = Join-Path $CaseRoot 'console-probe.json'
        $pidPath = Join-Path $CaseRoot 'console-probe.pid'
        $readyPath = Join-Path $CaseRoot 'console-probe.ready'
        $assemblyLiteral = $probeAssembly.Replace("'", "''")
        $reportLiteral = $probePath.Replace("'", "''")
        $pidLiteral = $pidPath.Replace("'", "''")
        $readyLiteral = $readyPath.Replace("'", "''")
        $script = "`$ErrorActionPreference = 'Stop'; Add-Type -Path '$assemblyLiteral'; " +
            "[IO.File]::WriteAllText('$readyLiteral', 'ready'); " +
            "`$deadline = [DateTime]::UtcNow.AddSeconds(10); " +
            "while (-not [IO.File]::Exists('$pidLiteral')) { " +
            "if ([DateTime]::UtcNow -ge `$deadline) { throw 'No owned helper PID was supplied.' }; Start-Sleep -Milliseconds 10 }; " +
            "`$snapshot = [SuperMenuTests.ConsoleProbe]::Read([uint32][IO.File]::ReadAllText('$pidLiteral')); " +
            "[IO.File]::WriteAllText('$reportLiteral', (`$snapshot | ConvertTo-Json -Compress))"
        $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $PSHOME 'pwsh.exe'))
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.ArgumentList.Add('-NoLogo')
        $start.ArgumentList.Add('-NoProfile')
        $start.ArgumentList.Add('-NonInteractive')
        $start.ArgumentList.Add('-EncodedCommand')
        $start.ArgumentList.Add([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script)))
        $probe = [Diagnostics.Process]::Start($start)
        try {
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            while (-not [IO.File]::Exists($readyPath)) {
                if ($probe.HasExited -or [DateTime]::UtcNow -ge $deadline) {
                    throw 'The read-only console probe did not initialize.'
                }
                Start-Sleep -Milliseconds 10
            }
            return [pscustomobject]@{ Process = $probe; PidPath = $pidPath; ReportPath = $probePath }
        } catch {
            if (-not $probe.HasExited) { $probe.Kill(); [void]$probe.WaitForExit(5000) }
            $probe.Dispose()
            throw
        }
    }

    function Read-OwnedConsole([Diagnostics.Process]$Helper, $Probe) {
        # The probe is ready before the two-second helper starts; CPU-heavy CI
        # startup cannot accidentally miss the short-lived console.
        $pendingPidPath = $Probe.PidPath + '.pending'
        [IO.File]::WriteAllText($pendingPidPath, [string]$Helper.Id)
        [IO.File]::Move($pendingPidPath, $Probe.PidPath)
        $probeProcess = $Probe.Process
        if (-not $probeProcess.WaitForExit(5000)) {
            $probeProcess.Kill()
            [void]$probeProcess.WaitForExit(5000)
            throw 'The read-only owned console probe timed out.'
        }
        $errors = $probeProcess.StandardError.ReadToEnd()
        if ($probeProcess.ExitCode -ne 0 -or -not [IO.File]::Exists($Probe.ReportPath)) {
            throw ('The owned console probe failed: ' + $errors)
        }
        return [IO.File]::ReadAllText($Probe.ReportPath) | ConvertFrom-Json
    }

    # HWND 1 can never be the owned SuperMenuWindow: valid_parent rejects these
    # requests before installed_inventory, resolve_winget, or any package command.
    # All four cases therefore exercise the real binary without touching software.
    $requests = @(
        @{ name = 'installation failure'; operation = 'install'; target = 'id:Google.Chrome'; arguments = '--install-package Google.Chrome --notify 1 --parent-pid ' + $PID + ' --request 1'; immediate = $false; probe = $true },
        @{ name = 'uninstallation failure'; operation = 'uninstall'; target = 'id:Google.Chrome'; arguments = '--uninstall-package id:Google.Chrome --notify 1 --parent-pid ' + $PID + ' --request 2'; immediate = $false; probe = $true },
        @{ name = 'malformed request failure'; operation = ''; target = ''; arguments = '--install-package Google.Chrome --notify 0 --parent-pid ' + $PID + ' --request 3'; immediate = $false; probe = $false },
        @{ name = 'diagnostic no-wait failure'; operation = ''; target = ''; arguments = '--install-package Google.Chrome --notify 1 --parent-pid ' + $PID + ' --request 4 --check-only --no-wait'; immediate = $true; probe = $false }
    )
    foreach ($request in $requests) {
        $caseRoot = Join-Path $workspace.Root ($request.name.Replace(' ', '-'))
        $helper = $null
        $probe = $null
        $timer = [Diagnostics.Stopwatch]::new()
        $case = [ordered]@{ case = $request.name; passed = $false }
        try {
            if ($request.probe) { $probe = Start-OwnedConsoleProbe -CaseRoot $caseRoot }
            $timer.Start()
            $helper = Start-OwnedHelper -Arguments $request.arguments -CaseRoot $caseRoot
            if ($request.probe) {
                $snapshot = Read-OwnedConsole -Helper $helper -Probe $probe
                $probe.Process.Dispose()
                $probe = $null
                $case.input_mode = $snapshot.InputMode
                Assert-Test -Context $context -Condition (($snapshot.InputMode -band 0x0040) -eq 0) -Name ($request.name + ': QuickEdit cannot pause the helper')
                Assert-Test -Context $context -Condition (($snapshot.InputMode -band 0x0080) -ne 0) -Name ($request.name + ': extended flags apply the QuickEdit setting')
                Assert-Test -Context $context -Condition ($snapshot.Text.Contains('Google Chrome', [StringComparison]::Ordinal)) -Name ($request.name + ': owned console contains the package heading')
                Assert-Test -Context $context -Condition (-not $snapshot.Text.Contains('Appuyez sur Entrée', [StringComparison]::Ordinal)) -Name ($request.name + ': no Enter prompt blocks automatic closure')
            }
            $closed = $helper.WaitForExit(6000)
            $timer.Stop()
            Assert-Test -Context $context -Condition $closed -Name ($request.name + ': helper closes without keyboard input')
            Assert-Test -Context $context -Condition ($helper.ExitCode -eq 1) -Name ($request.name + ': failure exit code is retained')
            $case.elapsed_ms = [Math]::Round($timer.Elapsed.TotalMilliseconds, 1)
            $case.exit_code = $helper.ExitCode
            if ($request.immediate) {
                Assert-Test -Context $context -Condition ($timer.Elapsed.TotalMilliseconds -lt 1800) -Name ($request.name + ': diagnostic skips the two-second display delay')
            } else {
                Assert-Test -Context $context -Condition ($timer.Elapsed.TotalMilliseconds -ge 1800 -and $timer.Elapsed.TotalMilliseconds -lt 6000) -Name ($request.name + ': failure remains briefly visible, then closes automatically')
            }
            $reportsDirectory = Join-Path $caseRoot 'SuperMenu/software-reports'
            $reports = @(if ([IO.Directory]::Exists($reportsDirectory)) { Get-ChildItem -LiteralPath $reportsDirectory -File -Filter '*.json' })
            if ($request.operation) {
                Assert-Test -Context $context -Condition ($reports.Count -eq 1) -Name ($request.name + ': one failure report survives console closure')
                $operationReport = [IO.File]::ReadAllText($reports[0].FullName) | ConvertFrom-Json
                $case.operation_report = $operationReport
                Assert-Test -Context $context -Condition ($operationReport.operation -ceq $request.operation) -Name ($request.name + ': report retains the operation')
                Assert-Test -Context $context -Condition ($operationReport.target -ceq $request.target) -Name ($request.name + ': report retains the canonical target')
                Assert-Test -Context $context -Condition ($operationReport.name -ceq 'Google Chrome') -Name ($request.name + ': report identifies the catalog application')
                Assert-Test -Context $context -Condition ($operationReport.status -ceq 'failure' -and $operationReport.result_code -eq 1 -and $operationReport.result_hex -ceq '0x00000001') -Name ($request.name + ': report retains the failure and exact result code')
                Assert-Test -Context $context -Condition ($operationReport.timestamp_ms -gt 0) -Name ($request.name + ': report has a timestamp')
                Assert-Test -Context $context -Condition ($operationReport.message.Contains("La fenêtre Super Menu n'est plus disponible.", [StringComparison]::Ordinal) -and [Text.Encoding]::UTF8.GetByteCount($operationReport.message) -le 8192) -Name ($request.name + ': report preserves the bounded validation failure')
                Assert-Test -Context $context -Condition ($operationReport.console_tail.Contains($operationReport.message.Trim(), [StringComparison]::Ordinal) -and [Text.Encoding]::UTF8.GetByteCount($operationReport.console_tail) -le 131072) -Name ($request.name + ': bounded console tail preserves the error after closure')
                Assert-Test -Context $context -Condition (-not $operationReport.console_tail.Contains('Appuyez sur Entrée', [StringComparison]::Ordinal)) -Name ($request.name + ': saved output contains no blocking Enter prompt')
            } else {
                Assert-Test -Context $context -Condition ($reports.Count -eq 0) -Name ($request.name + ': rejected or diagnostic request creates no operation report')
            }
            $case.passed = $true
        } finally {
            if ($null -ne $probe) {
                if (-not $probe.Process.HasExited) { $probe.Process.Kill(); [void]$probe.Process.WaitForExit(5000) }
                $probe.Process.Dispose()
            }
            if ($null -ne $helper) {
                if (-not $helper.HasExited) {
                    # Terminate only the exact process object started by this case.
                    $helper.Kill()
                    [void]$helper.WaitForExit(5000)
                }
                $helper.Dispose()
            }
            $cases.Add($case)
        }
    }
} catch {
    $failed = $true
    $failure = $_.Exception.Message
} finally {
    try { Remove-TestWorkspace -Workspace $workspace }
    catch { $failed = $true; $failure = 'Test cleanup failed: ' + $_.Exception.Message }
    $result = [ordered]@{ suite = 'native helper console'; passed = -not $failed; cases = $cases.ToArray(); checks = $checks.ToArray() }
    if ($failed) { $result.error = $failure }
    [void](Write-TestReport -Result $result -Path $Report)
    Write-Output ('Helper console: ' + $checks.Count + ' assertions, report: ' + [IO.Path]::GetFullPath($Report))
}
if ($failed) { throw ('Native helper console tests failed: ' + $failure) }
