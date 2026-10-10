# Shared Win32 test helpers. Every operation on a HWND checks its owning PID.
# These tests use messages targeted at their process; they never send desktop input.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not $IsWindows) {
    throw 'The native launcher tests require PowerShell 7 on Windows.'
}

if (-not ('SuperMenuTests.OwnedWindowApi' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace SuperMenuTests {
    public sealed class WindowData {
        public long Hwnd { get; set; }
        public int Id { get; set; }
        public string Class { get; set; }
        public string Text { get; set; }
        public bool Visible { get; set; }
        public int[] Rect { get; set; }
    }
    public sealed class ScrollData {
        public int Minimum { get; set; }
        public int Maximum { get; set; }
        public uint Page { get; set; }
        public int Position { get; set; }
        public int Bottom { get { return Math.Max(Minimum, Maximum - (int)Page + 1); } }
    }
    public sealed class ResourceData {
        public uint Gdi { get; set; }
        public uint User { get; set; }
    }
    public sealed class OwnedWindowApi {
        private readonly uint processId;
        private const uint TimeoutFlags = 0x0002 | 0x0020; // ABORTIFHUNG | ERRORONEXIT
        private const uint MessageTimeoutMs = 5000;
        private delegate bool EnumProc(IntPtr hwnd, IntPtr parameter);
        [StructLayout(LayoutKind.Sequential)] private struct Rect { public int L, T, R, B; }
        [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
        [StructLayout(LayoutKind.Sequential)] private struct ScrollInfo {
            public uint Size, Mask; public int Min, Max; public uint Page; public int Position, TrackPosition;
        }
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr hwnd, EnumProc callback, IntPtr parameter);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int length);
        [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool GetClientRect(IntPtr hwnd, out Rect bounds);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool GetWindowRect(IntPtr hwnd, out Rect bounds);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool GetScrollInfo(IntPtr hwnd, int bar, ref ScrollInfo info);
        [DllImport("user32.dll")] private static extern uint GetDpiForWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] private static extern uint GetGuiResources(IntPtr process, uint flags);
        [DllImport("kernel32.dll")] private static extern uint GetProcessId(IntPtr process);
        [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)] private static extern IntPtr SendMessageTimeout(
            IntPtr hwnd, uint message, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)] private static extern IntPtr SendTextTimeout(
            IntPtr hwnd, uint message, IntPtr wp, StringBuilder text, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll", EntryPoint="PostMessageW", SetLastError=true)] private static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wp, IntPtr lp);
        [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
        [DllImport("gdi32.dll", SetLastError=true)] private static extern bool BitBlt(IntPtr destination, int x, int y, int width, int height, IntPtr source, int sx, int sy, uint operation);

        public OwnedWindowApi(uint processId) { this.processId = processId; }
        private sealed class PhysicalPixels : IDisposable {
            private readonly IntPtr previous = SetThreadDpiAwarenessContext(new IntPtr(-4)); // PER_MONITOR_AWARE_V2
            public void Dispose() { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
        }
        public void AssertWindow(IntPtr hwnd) {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (hwnd == IntPtr.Zero || owner != processId || !IsWindow(hwnd))
                throw new InvalidOperationException("The HWND is not owned by the current test process.");
        }
        public IntPtr FindWindow(string className) {
            var matches = new List<IntPtr>();
            EnumProc callback = delegate(IntPtr hwnd, IntPtr unused) {
                uint owner;
                GetWindowThreadProcessId(hwnd, out owner);
                if (owner == processId) {
                    var text = new StringBuilder(256);
                    GetClassNameW(hwnd, text, text.Capacity);
                    if (text.ToString() == className) matches.Add(hwnd);
                }
                return true;
            };
            EnumWindows(callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            if (matches.Count > 1) throw new InvalidOperationException("The test process created multiple " + className + " windows.");
            return matches.Count == 0 ? IntPtr.Zero : matches[0];
        }
        public bool IsVisible(IntPtr hwnd) { AssertWindow(hwnd); return IsWindowVisible(hwnd); }
        public string ClassName(IntPtr hwnd) {
            AssertWindow(hwnd);
            var text = new StringBuilder(256);
            GetClassNameW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public IntPtr Send(IntPtr hwnd, uint message, long wp, long lp) {
            AssertWindow(hwnd);
            IntPtr result;
            if (SendMessageTimeout(hwnd, message, new IntPtr(wp), new IntPtr(lp), TimeoutFlags, MessageTimeoutMs, out result) == IntPtr.Zero)
                throw new InvalidOperationException("The test window did not answer message 0x" + message.ToString("X") + " within five seconds.");
            return result;
        }
        public void Post(IntPtr hwnd, uint message, long wp, long lp) {
            AssertWindow(hwnd);
            if (!PostMessage(hwnd, message, new IntPtr(wp), new IntPtr(lp))) throw new Win32Exception(Marshal.GetLastWin32Error());
        }
        public IntPtr SendText(IntPtr hwnd, uint message, long wp, string text) {
            AssertWindow(hwnd);
            if (text.Length > 32768) throw new ArgumentOutOfRangeException("text");
            var buffer = new StringBuilder(text, Math.Max(text.Length + 1, 1));
            IntPtr result;
            if (SendTextTimeout(hwnd, message, new IntPtr(wp), buffer, TimeoutFlags, MessageTimeoutMs, out result) == IntPtr.Zero)
                throw new InvalidOperationException("The test control did not answer a text message within five seconds.");
            return result;
        }
        public string ReadText(IntPtr hwnd) {
            int length = Send(hwnd, 0x000E, 0, 0).ToInt32(); // WM_GETTEXTLENGTH
            if (length < 0 || length > 32768) throw new InvalidOperationException("The test control text exceeds the bound.");
            AssertWindow(hwnd);
            var text = new StringBuilder(length + 1);
            IntPtr result;
            if (SendTextTimeout(hwnd, 0x000D, new IntPtr(length + 1), text, TimeoutFlags, MessageTimeoutMs, out result) == IntPtr.Zero)
                throw new InvalidOperationException("The test control did not return its text within five seconds.");
            return text.ToString();
        }
        public int[] Selection(IntPtr hwnd) {
            long selection = Send(hwnd, 0x00B0, 0, 0).ToInt64(); // EM_GETSEL, sufficient for these short fixtures
            return new int[] { (int)(selection & 0xffff), (int)((selection >> 16) & 0xffff) };
        }
        public void TypeCharacters(IntPtr hwnd, string text) {
            foreach (char character in text) Send(hwnd, 0x0102, character, 0); // UTF-16 WM_CHAR
        }
        public uint GetDpi(IntPtr hwnd) { AssertWindow(hwnd); return Math.Max(96u, GetDpiForWindow(hwnd)); }
        public int[] GetWindowBounds(IntPtr hwnd) {
            AssertWindow(hwnd);
            using (var dpi = new PhysicalPixels()) {
                Rect bounds;
                if (!GetWindowRect(hwnd, out bounds)) throw new Win32Exception(Marshal.GetLastWin32Error());
                return new int[] { bounds.L, bounds.T, bounds.R, bounds.B };
            }
        }
        public int[] GetClientBounds(IntPtr hwnd) {
            AssertWindow(hwnd);
            using (var dpi = new PhysicalPixels()) {
                Rect bounds; var origin = new Point();
                if (!GetClientRect(hwnd, out bounds) || !ClientToScreen(hwnd, ref origin)) throw new Win32Exception(Marshal.GetLastWin32Error());
                return new int[] { origin.X, origin.Y, origin.X + bounds.R - bounds.L, origin.Y + bounds.B - bounds.T };
            }
        }
        public void Resize(IntPtr hwnd, int width, int height) {
            AssertWindow(hwnd);
            if (width < 1 || height < 1 || width > 4096 || height > 4096) throw new ArgumentOutOfRangeException("size");
            using (var dpi = new PhysicalPixels()) {
                if (!SetWindowPos(hwnd, IntPtr.Zero, 0, 0, width, height, 0x0016)) throw new Win32Exception(Marshal.GetLastWin32Error()); // NOMOVE | NOZORDER | NOACTIVATE
            }
        }
        public ScrollData Scroll(IntPtr hwnd) {
            AssertWindow(hwnd);
            var info = new ScrollInfo { Size = (uint)Marshal.SizeOf(typeof(ScrollInfo)), Mask = 0x0017 };
            if (!GetScrollInfo(hwnd, 1, ref info)) throw new Win32Exception(Marshal.GetLastWin32Error());
            return new ScrollData { Minimum = info.Min, Maximum = info.Max, Page = info.Page, Position = info.Position };
        }
        public WindowData[] GetControls(IntPtr parent) {
            AssertWindow(parent);
            var handles = new List<IntPtr>();
            EnumProc callback = delegate(IntPtr hwnd, IntPtr unused) { handles.Add(hwnd); return true; };
            EnumChildWindows(parent, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            var result = new List<WindowData>();
            foreach (var hwnd in handles) {
                AssertWindow(hwnd);
                result.Add(new WindowData { Hwnd = hwnd.ToInt64(), Id = GetDlgCtrlID(hwnd), Class = ClassName(hwnd),
                    Text = ReadText(hwnd), Visible = IsVisible(hwnd), Rect = GetWindowBounds(hwnd) });
            }
            return result.ToArray();
        }
        public ResourceData GuiResources(IntPtr process) {
            if (GetProcessId(process) != processId) throw new InvalidOperationException("Cannot sample an unrelated process.");
            return new ResourceData { Gdi = GetGuiResources(process, 0), User = GetGuiResources(process, 1) };
        }
        public void CopyClientToDc(IntPtr hwnd, IntPtr destination) {
            AssertWindow(hwnd);
            using (var dpi = new PhysicalPixels()) {
                Rect bounds;
                if (!GetClientRect(hwnd, out bounds)) throw new Win32Exception(Marshal.GetLastWin32Error());
                // Captures are optional diagnostics. They do not send input or request focus.
                var source = GetDC(hwnd);
                try {
                    if (source == IntPtr.Zero || !BitBlt(destination, 0, 0, bounds.R, bounds.B, source, 0, 0, 0x00CC0020))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                } finally { if (source != IntPtr.Zero) ReleaseDC(hwnd, source); }
            }
        }
    }
}
'@
}

function New-TestWorkspace {
    $token = [Guid]::NewGuid().ToString('N')
    $path = Join-Path ([IO.Path]::GetTempPath()) ('SuperMenu-native-' + $token)
    [void][IO.Directory]::CreateDirectory($path)
    [IO.File]::WriteAllText((Join-Path $path '.supermenu-test-workspace'), $token)
    return [pscustomobject]@{ Root = [IO.Path]::GetFullPath($path); Token = $token }
}

function Remove-TestWorkspace {
    param([Parameter(Mandatory)]$Workspace)
    $path = [IO.Path]::GetFullPath($Workspace.Root)
    $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $marker = Join-Path $path '.supermenu-test-workspace'
    if (-not $path.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($path) -cne ('SuperMenu-native-' + $Workspace.Token) -or
        -not [IO.File]::Exists($marker) -or [IO.File]::ReadAllText($marker) -cne $Workspace.Token) {
        throw 'Refusing to remove a directory without the matching test-workspace sentinel.'
    }
    # One shell, literal path, verified exact owned target under the temporary directory.
    Remove-Item -LiteralPath $path -Recurse -Force
}

function Assert-Test {
    param([Parameter(Mandatory)]$Context, [Parameter(Mandatory)][bool]$Condition, [Parameter(Mandatory)][string]$Name)
    $Context.Checks.Add([pscustomobject]@{ name = $Name; passed = $Condition })
    if (-not $Condition) { throw ('Assertion failed: ' + $Name) }
}

function Wait-TestCondition {
    param([Parameter(Mandatory)]$Context, [Parameter(Mandatory)][scriptblock]$Condition,
        [Parameter(Mandatory)][string]$Message, [ValidateRange(1, 60)][int]$TimeoutSeconds = 10)
    $end = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    if ($Context.Deadline -lt $end) { $end = $Context.Deadline }
    do {
        if ($Context.Process.HasExited) { throw ('The test process exited: ' + $Message) }
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $end)
    throw ('Timed out: ' + $Message)
}

function Start-TestLauncher {
    param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Fixture,
        [ValidateSet('dark', 'light')][string]$Theme = 'dark',
        [ValidateSet('', 'fr', 'en')][string]$Language = '',
        [string]$LanguageDirectory = '',
        [switch]$FirstLanguagePicker,
        [ValidateRange(15, 300)][int]$TimeoutSeconds = 90)
    $exe = [IO.Path]::GetFullPath($Executable)
    $fixturePath = [IO.Path]::GetFullPath($Fixture)
    if (-not [IO.File]::Exists($exe) -or -not [IO.File]::Exists($fixturePath)) {
        throw 'The executable and JSON fixture must both exist.'
    }
    if ($Language -and -not $LanguageDirectory) {
        throw 'A test language requires an explicit isolated language-cache directory.'
    }
    if ($LanguageDirectory -and -not $Language) {
        throw 'A test language-cache directory requires an explicit language.'
    }
    if ($FirstLanguagePicker -and -not $LanguageDirectory) {
        throw 'Testing the first language picker requires an explicit isolated cache directory.'
    }
    if ($LanguageDirectory) {
        $LanguageDirectory = [IO.Path]::GetFullPath($LanguageDirectory)
        if (-not [IO.Directory]::Exists($LanguageDirectory)) {
            throw 'The isolated language-cache directory does not exist.'
        }
    }
    $workspace = New-TestWorkspace
    $context = $null
    try {
        $reportPath = Join-Path $workspace.Root 'native-smoke.json'
        $arguments = '--smoke-test --smoke-fixture "' + $fixturePath + '" --smoke-theme ' + $Theme +
            ' --smoke-timeout-ms ' + ($TimeoutSeconds * 1000) + ' --report "' + $reportPath + '"'
        if ($Language) {
            $arguments += ' --smoke-language ' + $Language + ' --smoke-language-dir "' + $LanguageDirectory + '"'
        }
        if ($FirstLanguagePicker) { $arguments += ' --smoke-first-run-language' }
        $process = Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden -PassThru
        $context = [pscustomobject]@{
            Process = $process; Pid = [uint32]$process.Id
            Api = [SuperMenuTests.OwnedWindowApi]::new([uint32]$process.Id)
            Main = [IntPtr]::Zero; Workspace = $workspace; ReportPath = $reportPath
            Deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
            Checks = [Collections.Generic.List[object]]::new(); Theme = $Theme
        }
        Wait-TestCondition -Context $context -Message 'the owned main window appears' -Condition {
            $context.Main = $context.Api.FindWindow('SuperMenuWindow')
            return $context.Main -ne [IntPtr]::Zero
        }
        return $context
    } catch {
        if ($null -ne $context) { Stop-TestLauncher -Context $context }
        else { Remove-TestWorkspace -Workspace $workspace }
        throw
    }
}

function Read-TestSnapshot {
    param([Parameter(Mandatory)]$Context)
    # A fixture-only message writes the current UI snapshot without closing the launcher.
    $acknowledgement = $Context.Api.Send($Context.Main, 0x800A, 0, 0)
    if ($acknowledgement.ToInt64() -ne 1) { throw 'The fixture snapshot message was not acknowledged; refusing to read a stale report.' }
    if (-not [IO.File]::Exists($Context.ReportPath)) { throw 'The native snapshot report was not created.' }
    return [IO.File]::ReadAllText($Context.ReportPath) | ConvertFrom-Json
}

function Complete-TestLauncher {
    param([Parameter(Mandatory)]$Context)
    $Context.Api.Post($Context.Main, 0x0113, 99, 0)
    if (-not $Context.Process.WaitForExit(10000)) { throw 'The owned smoke process did not exit within ten seconds.' }
    Assert-Test -Context $Context -Condition ($Context.Process.ExitCode -eq 0) -Name 'native smoke exits successfully'
    if (-not [IO.File]::Exists($Context.ReportPath)) { throw 'The native smoke report is missing.' }
    $native = [IO.File]::ReadAllText($Context.ReportPath) | ConvertFrom-Json
    Assert-Test -Context $Context -Condition ([bool]$native.fixture_mode) -Name 'isolated fixture mode is active'
    $expectedNames = @('all_providers', 'about_menu_entry', 'timer_menu_entry', 'timer_preview_without_start',
        'timer_invalid_duration', 'package_menu_entries', 'applications', 'default_applications', 'explicit_files',
        'install_catalog', 'remove_catalog', 'calculator', 'web_encoding', 'explicit_command',
        'keyboard_selection', 'escape_clears', 'system_actions')
    Assert-Test -Context $Context -Condition (@($native.checks).Count -eq $expectedNames.Count) -Name 'native smoke contains exactly the 17 expected assertions'
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($check in $native.checks) {
        $valid = $check -is [array] -and $check.Count -eq 2 -and $check[0] -is [string] -and $check[1] -is [bool]
        Assert-Test -Context $Context -Condition $valid -Name 'native smoke entry has the required [name, boolean] format'
        Assert-Test -Context $Context -Condition ($expectedNames -ccontains $check[0]) -Name ('native smoke has an expected assertion name: ' + $check[0])
        Assert-Test -Context $Context -Condition ($seen.Add($check[0])) -Name ('native smoke assertion name is unique: ' + $check[0])
        Assert-Test -Context $Context -Condition $check[1] -Name ('native smoke: ' + $check[0])
    }
    Assert-Test -Context $Context -Condition ($seen.Count -eq $expectedNames.Count) -Name 'every expected native smoke assertion is present exactly once'
    return $native
}

function Stop-TestLauncher {
    param([Parameter(Mandatory)]$Context)
    try {
        if (-not $Context.Process.HasExited) {
            try {
                $main = $Context.Api.FindWindow('SuperMenuWindow')
                if ($main -ne [IntPtr]::Zero) {
                    $Context.Api.Post($main, 0x0113, 99, 0)
                    [void]$Context.Process.WaitForExit(2000)
                }
            } catch { } # Cleanup still terminates only the process object created by this harness.
            if (-not $Context.Process.HasExited) {
                $Context.Process.Kill()
                [void]$Context.Process.WaitForExit(5000)
            }
        }
    } finally {
        $Context.Process.Dispose()
        Remove-TestWorkspace -Workspace $Context.Workspace
    }
}

function Save-TestWindow {
    param([Parameter(Mandatory)]$Context, [Parameter(Mandatory)][IntPtr]$Window,
        [Parameter(Mandatory)][string]$Path)
    Add-Type -AssemblyName System.Drawing
    $Context.Api.AssertWindow($Window)
    $bounds = $Context.Api.GetClientBounds($Window)
    $width = $bounds[2] - $bounds[0]
    $height = $bounds[3] - $bounds[1]
    if ($width -le 0 -or $height -le 0) { throw 'The owned window has no client area to capture.' }
    $target = [IO.Path]::GetFullPath($Path)
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target))
    $bitmap = [Drawing.Bitmap]::new($width, $height)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try { $Context.Api.CopyClientToDc($Window, $dc) }
            finally { $graphics.ReleaseHdc($dc) }
        } finally { $graphics.Dispose() }
        $bitmap.Save($target, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    return $target
}

function Write-TestReport {
    param([Parameter(Mandatory)]$Result, [string]$Path)
    $json = $Result | ConvertTo-Json -Depth 30
    if ($Path) {
        $absolute = [IO.Path]::GetFullPath($Path)
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($absolute))
        [IO.File]::WriteAllText($absolute, $json, [Text.UTF8Encoding]::new($false))
    }
    return $json
}
