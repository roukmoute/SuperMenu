<#
.SYNOPSIS
Exercises the actual native About window with an isolated, deterministic launcher index.
.EXAMPLE
pwsh -NoProfile -File tests/windows/about.ps1 -Executable target/release/supermenu.exe
.EXAMPLE
pwsh -NoProfile -File tests/windows/about.ps1 -Executable target/release/supermenu.exe -Capture -OutputDirectory test-results/about -Report test-results/about.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$Fixture = (Join-Path $PSScriptRoot '../fixtures/launcher.json'),
    [Alias('Version')][string]$ExpectedVersion,
    [ValidateSet('dark', 'light')][string[]]$Themes = @('dark', 'light'),
    [switch]$Capture,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [ValidateRange(30, 300)][int]$TimeoutSeconds = 90
)

. (Join-Path $PSScriptRoot 'support.ps1')

if (-not $Report) { $Report = Join-Path $OutputDirectory 'about.json' }

try {
    if (-not $ExpectedVersion) {
        $manifestPath = Join-Path $PSScriptRoot '../../Cargo.toml'
        $manifest = [IO.File]::ReadAllText([IO.Path]::GetFullPath($manifestPath))
        $match = [Regex]::Match($manifest, '(?ms)^\[package\]\s.*?^version\s*=\s*"([^"]+)"')
        if (-not $match.Success) { throw 'Could not read the package version from Cargo.toml; specify -ExpectedVersion.' }
        $ExpectedVersion = $match.Groups[1].Value
    }
    $fixtureItems = @([IO.File]::ReadAllText([IO.Path]::GetFullPath($Fixture)) | ConvertFrom-Json)
    $expectedApps = @($fixtureItems | Where-Object { $_.kind -eq 'Application' }).Count
    $expectedFiles = @($fixtureItems | Where-Object { $_.kind -in @('File', 'Folder') }).Count
    if ($expectedApps -eq 0 -or $expectedFiles -eq 0) {
        throw 'The About fixture must contain applications and files so both displayed counts can be verified.'
    }
} catch {
    [void](Write-TestReport -Result ([ordered]@{
        suite = 'native About window'; passed = $false; error = $_.Exception.Message
        themes = @(); captures_requested = [bool]$Capture
    }) -Path $Report)
    throw
}

function Open-TestAbout {
    param($Context)
    [void]$Context.Api.Send($Context.Main, 0x8009, 0, 0)
    $window = [IntPtr]::Zero
    Wait-TestCondition -Context $Context -Message 'About becomes visible after an explicit request' -Condition {
        $window = $Context.Api.FindWindow('SuperMenuAboutWindow')
        return $window -ne [IntPtr]::Zero -and $Context.Api.IsVisible($window)
    }
    # Scope inside a scriptblock is intentionally not used to return the handle.
    return $Context.Api.FindWindow('SuperMenuAboutWindow')
}

function Wait-TestAboutData {
    param($Context, [IntPtr]$Window)
    Wait-TestCondition -Context $Context -TimeoutSeconds 20 -Message 'About finishes collecting local system information' -Condition {
        $snapshot = Read-TestSnapshot -Context $Context
        if (-not $snapshot.about_loaded -or $snapshot.about_collecting) { return $false }
        $controls = @($Context.Api.GetControls($Window))
        $loading = @($controls | Where-Object { $_.Visible -and $_.Text -match '^Chargement' })
        return $controls.Count -gt 0 -and $loading.Count -eq 0 -and
            @($controls | Where-Object { $_.Text -ceq 'Exécutable Super Menu' }).Count -eq 1
    }
    return @($Context.Api.GetControls($Window))
}

function Assert-TestFooter {
    param($Context, [IntPtr]$Window, [object[]]$Controls, [string]$Stage)
    $client = $Context.Api.GetClientBounds($Window)
    $buttons = @($Controls | Where-Object { $_.Class -eq 'BUTTON' -and $_.Id -in @(3101, 3102) })
    Assert-Test -Context $Context -Condition ($buttons.Count -eq 2) -Name ($Stage + ': GitHub and Close buttons exist')
    foreach ($button in $buttons) {
        $Context.Api.AssertWindow([IntPtr]$button.Hwnd)
        $inside = $button.Visible -and $button.Rect[0] -ge $client[0] -and $button.Rect[1] -ge $client[1] -and
            $button.Rect[2] -le $client[2] -and $button.Rect[3] -le $client[3] -and
            $button.Rect[2] -gt $button.Rect[0] -and $button.Rect[3] -gt $button.Rect[1]
        Assert-Test -Context $Context -Condition $inside -Name ($Stage + ': ' + $button.Text + ' remains inside the client')
    }
    return $buttons
}

function Assert-TestDataPreserved {
    param($Context, [object[]]$Before, [object[]]$After, [string]$Stage)
    $beforeText = @($Before | Where-Object { $_.Class -eq 'STATIC' } | ForEach-Object { $_.Text } | Sort-Object)
    $afterText = @($After | Where-Object { $_.Class -eq 'STATIC' } | ForEach-Object { $_.Text } | Sort-Object)
    $same = $beforeText.Count -eq $afterText.Count -and
        @(Compare-Object -ReferenceObject $beforeText -DifferenceObject $afterText).Count -eq 0
    Assert-Test -Context $Context -Condition $same -Name ($Stage + ': all available metadata is preserved')
}

function Close-TestAbout {
    param($Context, [IntPtr]$Window, [ValidateSet('escape', 'x', 'button')][string]$Method)
    switch ($Method) {
        'escape' { $Context.Api.Post($Window, 0x0100, 27, 0) } # Routed through the app's native message loop.
        'x' { $Context.Api.Post($Window, 0x0010, 0, 0) }       # WM_CLOSE from the standard title-bar close action.
        'button' {
            $button = @($Context.Api.GetControls($Window) | Where-Object { $_.Class -eq 'BUTTON' -and $_.Id -eq 3102 })
            Assert-Test -Context $Context -Condition ($button.Count -eq 1) -Name 'the Close button can be found'
            [void]$Context.Api.Send([IntPtr]$button[0].Hwnd, 0x00F5, 0, 0) # BM_CLICK on the owned button, never GitHub.
        }
    }
    Wait-TestCondition -Context $Context -Message ($Method + ' hides About') -Condition {
        return -not $Context.Api.IsVisible($Window)
    }
    $Context.Api.AssertWindow($Window)
    Assert-Test -Context $Context -Condition $true -Name ($Method + ' hides and preserves the reusable window')
}

$themeResults = [Collections.Generic.List[object]]::new()
$failed = $false
$failure = $null
try {
    foreach ($theme in $Themes) {
        $context = $null
        $window = [IntPtr]::Zero
        $captures = [Collections.Generic.List[string]]::new()
        $themeResult = [ordered]@{ theme = $theme; passed = $false; checks = @(); captures = @() }
        try {
            $context = Start-TestLauncher -Executable $Executable -Fixture $Fixture -Theme $theme -TimeoutSeconds $TimeoutSeconds
            $beforeOpen = Read-TestSnapshot -Context $context
            Assert-Test -Context $context -Condition (-not $beforeOpen.about_created) -Name 'About is created lazily rather than during startup'
            Assert-Test -Context $context -Condition ([bool]$beforeOpen.fixture_mode) -Name 'the launcher starts with an isolated fixture'
            $window = Open-TestAbout -Context $context
            $firstHandle = $window.ToInt64()
            Assert-Test -Context $context -Condition ($context.Api.ReadText($window) -ceq 'Super Menu · À propos') -Name 'the native About caption is correct'
            $controls = @(Wait-TestAboutData -Context $context -Window $window)
            $texts = @($controls | Where-Object { $_.Class -eq 'STATIC' } | ForEach-Object { $_.Text })
            Assert-Test -Context $context -Condition ($texts -ccontains ('Version ' + $ExpectedVersion)) -Name 'About displays the current package version'
            foreach ($heading in @('Matériel', 'Windows', 'Activité')) {
                Assert-Test -Context $context -Condition ($texts -ccontains $heading) -Name ('About exposes the ' + $heading + ' card')
            }
            foreach ($label in @('Machine', 'Processeur', 'Graphiques', 'Écran', 'Mémoire', 'Disque système',
                'Édition', 'Version', 'Build', 'Architecture', 'Allumé depuis', 'Windows installé le', 'Exécutable Super Menu')) {
                Assert-Test -Context $context -Condition ($texts -ccontains $label) -Name ('About exposes the metadata field: ' + $label)
            }
            # Real hardware varies across contributors and CI; the native collection must finish,
            # and fields may explicitly read "Indisponible" when a device/API is unavailable.
            Assert-Test -Context $context -Condition ($texts -ccontains ($expectedApps.ToString() + ' applications')) -Name 'About shows the fixture application count'
            Assert-Test -Context $context -Condition ($texts -ccontains ($expectedFiles.ToString() + ' fichiers et dossiers')) -Name 'About shows the fixture file count'
            Assert-Test -Context $context -Condition ($texts -ccontains ($fixtureItems.Count.ToString() + " entrées dans l’index local")) -Name 'About shows the fixture index count'
            $binarySize = ([IO.FileInfo]::new([IO.Path]::GetFullPath($Executable))).Length
            Assert-Test -Context $context -Condition (@($texts | Where-Object { $_ -match ('\(' + $binarySize + ' octets\)$') }).Count -eq 1) -Name 'About reports the actual executable byte size'
            [void](Assert-TestFooter -Context $context -Window $window -Controls $controls -Stage 'default size')
            if ($Capture) { $captures.Add((Save-TestWindow -Context $context -Window $window -Path (Join-Path $OutputDirectory ('about-' + $theme + '.png')))) }

            # Logical sizes adapt to the process's DPI instead of assuming the developer's display.
            $dpi = $context.Api.GetDpi($window)
            $compactWidth = [int][Math]::Round(560 * $dpi / 96.0)
            $compactHeight = [int][Math]::Round(500 * $dpi / 96.0)
            $context.Api.Resize($window, $compactWidth, $compactHeight)
            $compactControls = @($context.Api.GetControls($window))
            Assert-TestDataPreserved -Context $context -Before $controls -After $compactControls -Stage 'compact size'
            $compactFooter = @(Assert-TestFooter -Context $context -Window $window -Controls $compactControls -Stage 'compact size')
            $content = @($compactControls | Where-Object { $_.Class -ceq 'SuperMenuAboutContent' })
            Assert-Test -Context $context -Condition ($content.Count -eq 1) -Name 'About exposes one native scrolling content window'
            $contentHandle = [IntPtr]$content[0].Hwnd
            $scrollBefore = $context.Api.Scroll($contentHandle)
            Assert-Test -Context $context -Condition ($scrollBefore.Bottom -gt $scrollBefore.Minimum) -Name 'compact About has scrollable overflow'
            if ($Capture) { $captures.Add((Save-TestWindow -Context $context -Window $window -Path (Join-Path $OutputDirectory ('about-' + $theme + '-compact.png')))) }
            [void]$context.Api.Send($contentHandle, 0x0115, 7, 0) # WM_VSCROLL, SB_BOTTOM
            $scrollAfter = $context.Api.Scroll($contentHandle)
            Assert-Test -Context $context -Condition ($scrollAfter.Position -eq $scrollBefore.Bottom) -Name 'native scrolling reaches the bottom'
            $bottomControls = @($context.Api.GetControls($window))
            Assert-TestDataPreserved -Context $context -Before $controls -After $bottomControls -Stage 'after scrolling'
            $bottomFooter = @(Assert-TestFooter -Context $context -Window $window -Controls $bottomControls -Stage 'after scrolling')
            foreach ($beforeButton in $compactFooter) {
                $afterButton = @($bottomFooter | Where-Object { $_.Id -eq $beforeButton.Id })[0]
                $same = @(Compare-Object -ReferenceObject $beforeButton.Rect -DifferenceObject $afterButton.Rect).Count -eq 0
                Assert-Test -Context $context -Condition $same -Name ('scrolling keeps the ' + $beforeButton.Text + ' footer fixed')
            }
            if ($Capture) { $captures.Add((Save-TestWindow -Context $context -Window $window -Path (Join-Path $OutputDirectory ('about-' + $theme + '-bottom.png')))) }

            # Reset size and warm the window before sampling resources. No screenshots or
            # background indexing are running during the repeated-close resource comparison.
            $window = Open-TestAbout -Context $context
            [void](Wait-TestAboutData -Context $context -Window $window)
            $warmResources = $context.Api.GuiResources($context.Process.Handle)
            foreach ($method in @('x', 'escape', 'button', 'escape', 'button', 'x')) {
                Close-TestAbout -Context $context -Window $window -Method $method
                $window = Open-TestAbout -Context $context
                Assert-Test -Context $context -Condition ($window.ToInt64() -eq $firstHandle) -Name ($method + ': reopening reuses the same native HWND')
                Assert-Test -Context $context -Condition ($context.Api.ReadText($window) -ceq 'Super Menu · À propos') -Name ($method + ': reopening preserves the native caption')
                [void](Wait-TestAboutData -Context $context -Window $window)
            }
            $finalResources = $context.Api.GuiResources($context.Process.Handle)
            Assert-Test -Context $context -Condition ($finalResources.Gdi -le $warmResources.Gdi + 4) -Name 'reopening has bounded GDI object growth (at most four)'
            Assert-Test -Context $context -Condition ($finalResources.User -le $warmResources.User + 4) -Name 'reopening has bounded USER object growth (at most four)'
            Assert-Test -Context $context -Condition ($finalResources.Gdi -lt 1024 -and $finalResources.User -lt 1024) -Name 'native GUI resource counts stay below the leak ceiling'
            Close-TestAbout -Context $context -Window $window -Method 'escape'
            $native = Complete-TestLauncher -Context $context
            Assert-Test -Context $context -Condition ([bool]$native.about_created -and [bool]$native.about_loaded) -Name 'native report confirms About creation and collection'
            Assert-Test -Context $context -Condition ([bool]$native.indexing_finished) -Name 'the fixture index is ready'
            Assert-Test -Context $context -Condition ($native.fixture_activation_attempts -eq 0) -Name 'the About tests do not request external activation'
            $themeResult.passed = $true
            $themeResult.resources = [ordered]@{ warm = $warmResources; final = $finalResources; permitted_growth = 4 }
            $themeResult.native_smoke = $native
        } catch {
            $failed = $true
            $failure = $_.Exception.Message
            $themeResult.error = $failure
            if ($Capture -and $null -ne $context -and -not $context.Process.HasExited -and $window -ne [IntPtr]::Zero) {
                try { $captures.Add((Save-TestWindow -Context $context -Window $window -Path (Join-Path $OutputDirectory ('about-' + $theme + '-failure.png')))) }
                catch { $themeResult.capture_error = $_.Exception.Message }
            }
        } finally {
            if ($null -ne $context) {
                $themeResult.checks = $context.Checks.ToArray()
                try { Stop-TestLauncher -Context $context }
                catch {
                    $failed = $true
                    $failure = 'Test cleanup failed: ' + $_.Exception.Message
                    $themeResult.passed = $false
                    $themeResult.cleanup_error = $failure
                }
            }
            $themeResult.captures = $captures.ToArray()
            $themeResults.Add($themeResult)
        }
        if ($failed) { break }
    }
} finally {
    $result = [ordered]@{
        suite = 'native About window'; passed = -not $failed; expected_version = $ExpectedVersion
        fixture_counts = [ordered]@{ applications = $expectedApps; files = $expectedFiles; entries = $fixtureItems.Count }
        themes = $themeResults.ToArray(); captures_requested = [bool]$Capture
    }
    [void](Write-TestReport -Result $result -Path $Report)
    $checkCount = @($themeResults | ForEach-Object { $_.checks } | ForEach-Object { $_ }).Count
    Write-Output ('About: ' + $checkCount + ' assertions, ' + $themeResults.Count + ' theme(s), report: ' + [IO.Path]::GetFullPath($Report))
}
if ($failed) { throw ('Native About tests failed: ' + $failure) }
