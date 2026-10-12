#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$Fixture = (Join-Path $PSScriptRoot '../tests/fixtures/launcher.json'),
    [string]$Report = (Join-Path $PSScriptRoot '../artifacts/performance.json'),
    [ValidateRange(3, 30)][int]$StartupRuns = 9,
    [ValidateRange(20, 5000)][int]$Rounds = 300,
    [ValidateRange(1, 500)][int]$WarmupRounds = 30,
    [ValidateSet('dark', 'light')][string]$Theme = 'dark',
    [switch]$WindowPaint,
    [ValidateCount(2, 64)][string[]]$Queries = @('@ steam', '@ stream', '@ gamespy', '@ telegram', '@ no-match', '/ steam', '@ ste', '@ ')
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../tests/windows/support.ps1')

if (-not ('SuperMenuPerformance.OwnedMeasurement' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

namespace SuperMenuPerformance {
    // Timing stays inside C# to exclude PowerShell dispatch and pipeline costs.
    // Every message checks the exact process that this script just started.
    public sealed class OwnedMeasurement : IDisposable {
        private readonly uint processId;
        private readonly IntPtr main, edit;
        private readonly bool windowPaint;
        private IntPtr dc, bitmap, previousBitmap;
        private bool disposed;
        private const uint TimeoutFlags = 0x0002 | 0x0020;
        // RDW_INVALIDATE | RDW_NOERASE | RDW_NOCHILDREN | RDW_UPDATENOW.
        private const uint WindowPaintFlags = 0x0161;
        [StructLayout(LayoutKind.Sequential)] private struct Rect { public int L, T, R, B; }
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsChild(IntPtr parent, IntPtr child);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool GetClientRect(IntPtr hwnd, out Rect bounds);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool RedrawWindow(IntPtr hwnd, IntPtr bounds, IntPtr region, uint flags);
        [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
        [DllImport("gdi32.dll", SetLastError=true)] private static extern IntPtr CreateCompatibleDC(IntPtr dc);
        [DllImport("gdi32.dll", SetLastError=true)] private static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int width, int height);
        [DllImport("gdi32.dll", SetLastError=true)] private static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
        [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr obj);
        [DllImport("gdi32.dll")] private static extern bool DeleteDC(IntPtr dc);
        [DllImport("gdi32.dll")] private static extern bool GdiFlush();
        [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)]
        private static extern IntPtr SendMessageTimeout(IntPtr hwnd, uint message, IntPtr wp, IntPtr lp,
            uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern IntPtr SendTextTimeout(IntPtr hwnd, uint message, IntPtr wp, StringBuilder text,
            uint flags, uint timeout, out IntPtr result);

        public int Width { get; private set; }
        public int Height { get; private set; }

        public OwnedMeasurement(uint pid, IntPtr mainWindow, IntPtr editControl, bool measureWindowPaint) {
            processId = pid; main = mainWindow; edit = editControl; windowPaint = measureWindowPaint;
            AssertWindow(main); AssertWindow(edit);
            if (!IsChild(main, edit)) throw new InvalidOperationException("The EDIT is not a child of the owned launcher.");
            IntPtr previousDpi = SetThreadDpiAwarenessContext(new IntPtr(-4));
            IntPtr source = IntPtr.Zero;
            try {
                Rect bounds;
                if (!GetClientRect(main, out bounds)) throw new Win32Exception(Marshal.GetLastWin32Error());
                Width = bounds.R - bounds.L; Height = bounds.B - bounds.T;
                if (Width < 1 || Height < 1 || Width > 4096 || Height > 4096)
                    throw new InvalidOperationException("The owned client dimensions exceed the measurement bound.");
                if (windowPaint) return;
                source = GetDC(main);
                if (source == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                dc = CreateCompatibleDC(source);
                if (dc == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                bitmap = CreateCompatibleBitmap(source, Width, Height);
                if (bitmap == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                previousBitmap = SelectObject(dc, bitmap);
                if (previousBitmap == IntPtr.Zero || previousBitmap == new IntPtr(-1)) {
                    previousBitmap = IntPtr.Zero;
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
            } catch { Dispose(); throw; }
            finally {
                if (source != IntPtr.Zero) ReleaseDC(main, source);
                if (previousDpi != IntPtr.Zero) SetThreadDpiAwarenessContext(previousDpi);
            }
        }

        private void AssertWindow(IntPtr hwnd) {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (disposed || hwnd == IntPtr.Zero || owner != processId || !IsWindow(hwnd))
                throw new InvalidOperationException("Refusing to measure a window outside the owned process.");
        }

        private void Paint() {
            AssertWindow(main);
            if (windowPaint) {
                if (!RedrawWindow(main, IntPtr.Zero, IntPtr.Zero, WindowPaintFlags))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
            } else {
                IntPtr result;
                if (SendMessageTimeout(main, 0x0318, dc, new IntPtr(4), TimeoutFlags, 5000, out result) == IntPtr.Zero)
                    throw new InvalidOperationException("The owned WM_PRINTCLIENT did not finish within five seconds.");
            }
            if (!GdiFlush()) throw new Win32Exception(Marshal.GetLastWin32Error());
        }

        private void SetQuery(StringBuilder query) {
            AssertWindow(edit);
            IntPtr result;
            if (SendTextTimeout(edit, 0x000C, IntPtr.Zero, query, TimeoutFlags, 5000, out result) == IntPtr.Zero)
                throw new InvalidOperationException("The owned WM_SETTEXT did not finish within five seconds.");
        }

        public double[] PaintOnly(int rounds, int warmup) { return Measure(rounds, warmup, null); }
        public double[] SearchAndPaint(int rounds, int warmup, string[] queries) {
            if (queries == null || queries.Length < 2) throw new ArgumentException("Use at least two distinct queries.");
            var buffers = new StringBuilder[queries.Length];
            for (int i = 0; i < queries.Length; i++) {
                if (queries[i] == null || queries[i].Length > 128) throw new ArgumentOutOfRangeException("queries");
                buffers[i] = new StringBuilder(queries[i], queries[i].Length + 1);
            }
            return Measure(rounds, warmup, buffers);
        }

        private double[] Measure(int rounds, int warmup, StringBuilder[] queries) {
            if (rounds < 1 || rounds > 5000 || warmup < 1 || warmup > 500) throw new ArgumentOutOfRangeException("rounds");
            var samples = new double[rounds];
            IntPtr previousDpi = SetThreadDpiAwarenessContext(new IntPtr(-4));
            try {
                for (int i = -warmup; i < rounds; i++) {
                    long started = Stopwatch.GetTimestamp();
                    if (queries != null) SetQuery(queries[(i + warmup) % queries.Length]);
                    Paint();
                    long elapsed = Stopwatch.GetTimestamp() - started;
                    if (i >= 0) samples[i] = elapsed * 1000.0 / Stopwatch.Frequency;
                }
                return samples;
            } finally { if (previousDpi != IntPtr.Zero) SetThreadDpiAwarenessContext(previousDpi); }
        }

        public void Dispose() {
            if (disposed) return;
            disposed = true;
            if (dc != IntPtr.Zero && previousBitmap != IntPtr.Zero) SelectObject(dc, previousBitmap);
            if (bitmap != IntPtr.Zero) DeleteObject(bitmap);
            if (dc != IntPtr.Zero) DeleteDC(dc);
            dc = bitmap = previousBitmap = IntPtr.Zero;
        }
    }
}
'@
}

function Get-MeasurementStatistics([double[]]$Samples, [string]$Unit) {
    if ($Samples.Count -eq 0) { throw 'A measurement must contain at least one sample.' }
    $sorted = [double[]]$Samples.Clone()
    [Array]::Sort($sorted)
    $middle = [int][Math]::Floor($sorted.Length / 2)
    $median = if ($sorted.Length % 2) { $sorted[$middle] } else { ($sorted[$middle - 1] + $sorted[$middle]) / 2 }
    return [ordered]@{
        unit = $Unit; count = $Samples.Length
        median = $median; p95 = $sorted[[int][Math]::Ceiling($sorted.Length * 0.95) - 1]
        minimum = $sorted[0]; maximum = $sorted[-1]; samples = $Samples
    }
}

$Executable = [IO.Path]::GetFullPath($Executable)
$Fixture = [IO.Path]::GetFullPath($Fixture)
if (-not [IO.File]::Exists($Executable) -or -not [IO.File]::Exists($Fixture)) {
    throw 'The executable and the fixture must both exist.'
}
$fixtureItems = @([IO.File]::ReadAllText($Fixture) | ConvertFrom-Json)
if ($fixtureItems.Count -lt 1 -or $fixtureItems.Count -gt 256) { throw 'Use a fixture containing 1 to 256 items.' }
if (@($fixtureItems | Where-Object { -not ([string]$_.target).StartsWith('fixture://', [StringComparison]::Ordinal) }).Count) {
    throw 'Every performance fixture target must use fixture:// placeholders.'
}

$startup = [Collections.Generic.List[object]]::new()
$context = $null
$measurement = $null
# The timed loop cycles through these queries; accented ones exercise the non-ASCII normalization path.
$queries = [string[]]$Queries
try {
    for ($run = 0; $run -lt $StartupRuns; $run++) {
        $context = Start-TestLauncher -Executable $Executable -Fixture $Fixture -Theme $Theme
        Wait-TestCondition -Context $context -Message 'the first native frame and isolated fixture are ready' -Condition {
            $snapshot = Read-TestSnapshot -Context $context
            return [bool]$snapshot.fixture_mode -and [bool]$snapshot.indexing_finished -and $snapshot.first_frame_ms -gt 0
        }
        $snapshot = Read-TestSnapshot -Context $context
        $startup.Add([ordered]@{
            run = $run + 1; first_frame_ms = [double]$snapshot.first_frame_ms
            private_bytes = [long]$snapshot.private_bytes; working_set_bytes = [long]$snapshot.working_set_bytes
        })
        if ($run -lt $StartupRuns - 1) { Stop-TestLauncher -Context $context; $context = $null }
    }

    $controls = @($context.Api.GetControls($context.Main) | Where-Object { $_.Class -eq 'Edit' -or $_.Class -eq 'EDIT' })
    if ($controls.Count -ne 1) { throw 'The owned launcher must have exactly one native search EDIT.' }
    $edit = [IntPtr]$controls[0].Hwnd
    [void]$context.Api.Send($context.Main, 0x8002, 0, 0) # Populate fixture action cache; no machine scan.
    [void]$context.Api.SendText($edit, 0x000C, 0, '@ steam')
    $snapshot = Read-TestSnapshot -Context $context
    if ($snapshot.query -cne '@ steam' -or @($snapshot.items | Where-Object { $_.kind -ne 'Application' }).Count) {
        throw 'WM_SETTEXT must synchronously update the actual application search before timing.'
    }
    $measurement = [SuperMenuPerformance.OwnedMeasurement]::new($context.Pid, $context.Main, $edit, [bool]$WindowPaint)
    $paintSamples = $measurement.PaintOnly($Rounds, $WarmupRounds)
    $searchSamples = $measurement.SearchAndPaint($Rounds, $WarmupRounds, $queries)
    $final = Read-TestSnapshot -Context $context
    $expectedQuery = $queries[($Rounds + $WarmupRounds - 1) % $queries.Length]
    if ($final.query -cne $expectedQuery) { throw 'The timed loop did not update the actual query to its final value.' }
    $resources = $context.Api.GuiResources($context.Process.Handle)
    $paintLimitation = if ($WindowPaint) {
        'RedrawWindow synchronously executes the main window WM_PAINT, including its drawing buffer and client copy; it excludes the child EDIT and DWM screen presentation.'
    } else {
        'WM_PRINTCLIENT renders the main client to an offscreen bitmap; it excludes the child EDIT and DWM screen presentation.'
    }
    $result = [ordered]@{
        schema_version = 1; measured_at_utc = [DateTime]::UtcNow.ToString('o')
        executable = [ordered]@{ path = $Executable; bytes = [IO.FileInfo]::new($Executable).Length; sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash }
        fixture = [ordered]@{ path = $Fixture; items = $fixtureItems.Count; sha256 = (Get-FileHash -LiteralPath $Fixture -Algorithm SHA256).Hash }
        environment = [ordered]@{ os = [Environment]::OSVersion.VersionString; logical_processors = [Environment]::ProcessorCount; process_architecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString(); powershell = $PSVersionTable.PSVersion.ToString(); dpi = $context.Api.GetDpi($context.Main); client_width = $measurement.Width; client_height = $measurement.Height; theme = $Theme; language = $final.language }
        settings = [ordered]@{ startup_runs = $StartupRuns; rounds = $Rounds; warmup_rounds = $WarmupRounds; queries = $queries; window_paint = [bool]$WindowPaint; gdi_dc_and_bitmap_reused = -not [bool]$WindowPaint }
        startup = [ordered]@{
            first_frame_ms = Get-MeasurementStatistics -Samples @($startup | ForEach-Object { $_.first_frame_ms }) -Unit 'ms'
            private_bytes = Get-MeasurementStatistics -Samples @($startup | ForEach-Object { $_.private_bytes }) -Unit 'bytes'
            working_set_bytes = Get-MeasurementStatistics -Samples @($startup | ForEach-Object { $_.working_set_bytes }) -Unit 'bytes'
            runs = $startup.ToArray()
        }
        warm_ui = [ordered]@{
            paint_only = Get-MeasurementStatistics -Samples $paintSamples -Unit 'ms'
            search_and_paint = Get-MeasurementStatistics -Samples $searchSamples -Unit 'ms'
            private_bytes = [long]$final.private_bytes; working_set_bytes = [long]$final.working_set_bytes
            gdi_objects = $resources.Gdi; user_objects = $resources.User; final_query = $final.query
        }
        limitations = @(
            'First-frame timing starts inside ui::run; it excludes process creation, loader time and time to displayed DWM pixels.',
            'Startup samples are repeated launches on one machine with uncontrolled, usually warm Windows file and DLL caches.',
            'Warm UI timings include owned-window checks, synchronous cross-process Win32 messages and caller GdiFlush; they are not pure in-process Rust timings.',
            $paintLimitation,
            'The small isolated fixture measures UI work; use --benchmark separately for the 20,000-entry search index.',
            'These measurements have no fixed pass/fail performance threshold; compare identical fixtures, DPI, theme and machine conditions.'
        )
    }
    Write-TestReport -Result $result -Path $Report
}
finally {
    if ($null -ne $measurement) { $measurement.Dispose() }
    if ($null -ne $context) { Stop-TestLauncher -Context $context }
}
