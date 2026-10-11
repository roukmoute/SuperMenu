param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$Fixture = (Join-Path $PSScriptRoot '../fixtures/launcher.json'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [ValidateSet('dark', 'light')][string]$Theme = 'dark',
    [switch]$Capture
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'support.ps1')

$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $Report) { $Report = Join-Path $OutputDirectory 'launcher.json' }
$context = Start-TestLauncher -Executable $Executable -Fixture $Fixture -Theme $Theme
$cases = [Collections.Generic.List[object]]::new()
$captures = [Collections.Generic.List[string]]::new()

try {
    $controls = @($context.Api.GetControls($context.Main) | Where-Object { $_.Class -eq 'Edit' -or $_.Class -eq 'EDIT' })
    Assert-Test -Context $context -Condition ($controls.Count -eq 1) -Name 'one native search EDIT exists'
    $edit = [IntPtr]$controls[0].Hwnd

    function Set-OwnQuery([string]$Text) {
        # WM_SETTEXT only clears the control; search text travels through native WM_CHAR/EN_CHANGE.
        [void]$context.Api.SendText($edit, 0x000C, 0, '')
        $context.Api.TypeCharacters($edit, $Text)
    }

    function Assert-OwnResults([string]$Name, [string]$Query, [string[]]$Ids, [string[]]$Kinds) {
        $snapshot = Read-TestSnapshot -Context $context
        $actualIds = @($snapshot.items | ForEach-Object { $_.id })
        $expectedIds = @($Ids)
        $cases.Add([ordered]@{ case = $Name; query = $snapshot.query; selected = $snapshot.selected; section = $snapshot.section; expected_ids = $expectedIds; result_ids = $actualIds })
        $sameIds = $actualIds.Count -eq $expectedIds.Count -and
            [string]::Join(';', @($actualIds | Sort-Object)) -ceq [string]::Join(';', @($expectedIds | Sort-Object))
        Assert-Test -Context $context -Condition ($snapshot.query -ceq $Query) -Name ($Name + ': EN_CHANGE updates the query')
        Assert-Test -Context $context -Condition $sameIds -Name ($Name + ': exact visible result IDs')
        Assert-Test -Context $context -Condition (@($snapshot.items | Where-Object { $_.kind -notin $Kinds }).Count -eq 0) -Name ($Name + ': provider filters result kinds')
        return $snapshot
    }

    Wait-TestCondition -Context $context -Message 'the initialized fixture accepts read-only UI snapshots' -Condition {
        try { return [bool](Read-TestSnapshot -Context $context).fixture_mode }
        catch { return $false }
    }
    # Populate the package-action cache from the fixture through the normal INDEX notification.
    [void]$context.Api.Send($context.Main, 0x8002, 0, 0)
    $initial = Read-TestSnapshot -Context $context
    Assert-Test -Context $context -Condition ([bool]$initial.fixture_mode) -Name 'the UI uses the isolated fixture index'
    Assert-Test -Context $context -Condition ([bool]$initial.indexing_finished) -Name 'the fixture is ready without a machine scan'
    Assert-Test -Context $context -Condition ($initial.applications -eq 5 -and $initial.files -eq 3 -and $initial.index_items -eq 8) -Name 'the fixture has five applications and three file entries'

    # Inspect the application's current owner-drawn result list, not a second ranking implementation.
    Set-OwnQuery 'steam'
    $steam = Assert-OwnResults 'default applications' 'steam' @('fixture:steam', 'fixture:steam-support', 'uninstall:id:Valve.Steam') @('Application', 'Uninstall')
    Assert-Test -Context $context -Condition ($steam.items[0].id -ceq 'fixture:steam') -Name 'the exact Steam application is the first result'
    Assert-Test -Context $context -Condition (@($steam.items | Where-Object { $_.id -in @('fixture:telegram-uninstaller', 'fixture:streamfab', 'fixture:gamespy') }).Count -eq 0) -Name 'Steam excludes unrelated names and metadata paths'

    Set-OwnQuery '@ steam'
    [void](Assert-OwnResults 'explicit applications' '@ steam' @('fixture:steam', 'fixture:steam-support') @('Application'))
    Set-OwnQuery '- steam'
    [void](Assert-OwnResults 'explicit removal search without activation' '- steam' @('uninstall:id:Valve.Steam') @('Uninstall'))
    Set-OwnQuery '+ obsidian'
    [void](Assert-OwnResults 'explicit installation search without activation' '+ obsidian' @('install:Obsidian.Obsidian') @('Install'))
    # An absent Chrome must offer installation from both normal and explicit queries.
    # Inspect the real UI rows without activating a program or a package operation.
    Set-OwnQuery 'chrome'
    $chrome = Assert-OwnResults 'absent Chrome offers installation' 'chrome' @('install:Google.Chrome') @('Install')
    Assert-Test -Context $context -Condition ($chrome.items[0].title -ceq 'Google Chrome') -Name 'absent Chrome has the exact catalog application title'
    Assert-Test -Context $context -Condition (([string]$chrome.items[0].subtitle).StartsWith('Installer ·', [StringComparison]::Ordinal)) -Name 'absent Chrome visibly offers installation in the default search'
    Set-OwnQuery '+ chrome'
    $chromeInstall = Assert-OwnResults 'explicit absent Chrome installation' '+ chrome' @('install:Google.Chrome') @('Install')
    Assert-Test -Context $context -Condition ($chromeInstall.items[0].title -ceq 'Google Chrome') -Name 'explicit Chrome installation has the exact catalog application title'
    Assert-Test -Context $context -Condition (([string]$chromeInstall.items[0].subtitle).StartsWith('Installer ·', [StringComparison]::Ordinal)) -Name 'explicit Chrome installation has the installation subtitle'
    Set-OwnQuery '@ chrome'
    [void](Assert-OwnResults 'absent Chrome has no launchable application' '@ chrome' @() @('Application'))
    Set-OwnQuery 'path-only-token'
    [void](Assert-OwnResults 'target metadata is not an application name' 'path-only-token' @() @('Application'))
    Set-OwnQuery 'zebra'
    [void](Assert-OwnResults 'files require an explicit prefix' 'zebra' @() @('Application'))
    Set-OwnQuery '/ zebra'
    [void](Assert-OwnResults 'explicit file search' '/ zebra' @('fixture:zebra-document') @('File'))
    Set-OwnQuery '/ steam'
    [void](Assert-OwnResults 'files and folders share the file provider' '/ steam' @('fixture:steam-notes', 'fixture:steam-backups') @('File', 'Folder'))

    # The fixture-only hook sets Ctrl in the application's UI thread, sends the real
    # WM_KEYDOWN to its subclassed EDIT, then restores that thread's keyboard state.
    # No foreground window, global input event, or interactive desktop is required.
    $wordCases = @(
        @{ name = 'whole word'; input = 'hello world'; start = 11; end = 11; expected = 'hello '; caret = 6 },
        @{ name = 'trailing spaces'; input = 'hello world   '; start = 14; end = 14; expected = 'hello '; caret = 6 },
        @{ name = 'accented word'; input = 'bonjour café'; start = 12; end = 12; expected = 'bonjour '; caret = 8 },
        @{ name = 'surrogate pair'; input = 'bonjour 🦀'; start = 10; end = 10; expected = 'bonjour '; caret = 8 },
        @{ name = 'middle of word'; input = 'one three tail'; start = 6; end = 6; expected = 'one ree tail'; caret = 4 },
        @{ name = 'selected word'; input = 'hello world'; start = 6; end = 11; expected = 'hello '; caret = 6 },
        @{ name = 'reversed selection'; input = 'hello world'; start = 11; end = 6; expected = 'hello '; caret = 6 },
        @{ name = 'selected surrogate pair'; input = 'first 🦀 tail'; start = 6; end = 8; expected = 'first  tail'; caret = 6 },
        @{ name = 'start of input'; input = 'hello'; start = 0; end = 0; expected = 'hello'; caret = 0 },
        @{ name = 'empty input'; input = ''; start = 0; end = 0; expected = ''; caret = 0 },
        @{ name = 'search refresh'; input = 'steam junk'; start = 10; end = 10; expected = 'steam '; caret = 6 }
    )
    foreach ($case in $wordCases) {
        [void]$context.Api.SendText($edit, 0x000C, 0, $case.input)
        [void]$context.Api.Send($edit, 0x00B1, $case.start, $case.end) # EM_SETSEL
        $handled = $context.Api.Send($context.Main, 0x800B, 0, 0).ToInt64()
        Assert-Test -Context $context -Condition ($handled -eq 1) -Name ('Ctrl+Backspace hook: ' + $case.name)
        Assert-Test -Context $context -Condition ($context.Api.ReadText($edit) -ceq $case.expected) -Name ('Ctrl+Backspace deletes: ' + $case.name)
        $selection = $context.Api.Selection($edit)
        Assert-Test -Context $context -Condition ($selection[0] -eq $case.caret -and $selection[1] -eq $case.caret) -Name ('Ctrl+Backspace caret: ' + $case.name)
        # TranslateMessage can deliver DEL after Ctrl has been released. It must not insert a character.
        [void]$context.Api.Send($edit, 0x0102, 0x7F, 0)
        Assert-Test -Context $context -Condition ($context.Api.ReadText($edit) -ceq $case.expected) -Name ('translated DEL is suppressed: ' + $case.name)
        $afterDelete = Read-TestSnapshot -Context $context
        Assert-Test -Context $context -Condition ($afterDelete.query -ceq $case.expected) -Name ('Ctrl+Backspace sends EN_CHANGE: ' + $case.name)
        if ($case.name -eq 'search refresh') {
            [void](Assert-OwnResults 'word deletion refreshes application results' $case.expected @('fixture:steam', 'fixture:steam-support', 'uninstall:id:Valve.Steam') @('Application', 'Uninstall'))
        }
        if ($case.input -cne $case.expected) {
            $undo = $context.Api.Send($edit, 0x00C7, 0, 0).ToInt64() # EM_UNDO
            Assert-Test -Context $context -Condition ($undo -ne 0 -and $context.Api.ReadText($edit) -ceq $case.input) -Name ('Undo restores Ctrl+Backspace: ' + $case.name)
            $afterUndo = Read-TestSnapshot -Context $context
            Assert-Test -Context $context -Condition ($afterUndo.query -ceq $case.input) -Name ('Undo refreshes the query: ' + $case.name)
        }
        $cases.Add([ordered]@{ case = 'Ctrl+Backspace: ' + $case.name; input = $case.input; selection = @($case.start, $case.end); expected = $case.expected; caret = $case.caret })
    }

    [void]$context.Api.SendText($edit, 0x000C, 0, 'hello')
    [void]$context.Api.Send($edit, 0x00B1, 5, 5)
    [void]$context.Api.Send($edit, 0x0102, 8, 0)
    Assert-Test -Context $context -Condition ($context.Api.ReadText($edit) -ceq 'hell') -Name 'ordinary Backspace still removes one character'

    function Assert-OwnPrefix([string]$Name, [string]$Prefix) {
        Assert-Test -Context $context -Condition ($context.Api.ReadText($edit) -ceq $Prefix) -Name ($Name + ': navigation inserts the prefix')
        $selection = $context.Api.Selection($edit)
        Assert-Test -Context $context -Condition ($selection[0] -eq $Prefix.Length -and $selection[1] -eq $Prefix.Length) -Name ($Name + ': caret follows the prefix')
        $suffix = 'é🦀'
        $context.Api.TypeCharacters($edit, $suffix)
        $expected = $Prefix + $suffix
        Assert-Test -Context $context -Condition ($context.Api.ReadText($edit) -ceq $expected) -Name ($Name + ': Unicode input appends after the prefix')
        $selection = $context.Api.Selection($edit)
        Assert-Test -Context $context -Condition ($selection[0] -eq $expected.Length -and $selection[1] -eq $expected.Length) -Name ($Name + ': caret uses UTF-16 offsets')
        $snapshot = Read-TestSnapshot -Context $context
        Assert-Test -Context $context -Condition ($snapshot.query -ceq $expected) -Name ($Name + ': typing refreshes the query')
        $cases.Add([ordered]@{ case = $Name; prefix = $Prefix; typed = $expected; caret = $selection })
    }

    # Return is sent only after observing that the selected row is Navigation.
    # These rows insert prefixes; the test never activates a program or a package action.
    foreach ($prefix in @('@ ', '+ ', '- ', '/ ', '= ', 'minuteur ', '? ', '> ', ': ', '~ ', '. ', '! ')) {
        Set-OwnQuery ''
        $homeSnapshot = Read-TestSnapshot -Context $context
        $position = -1
        for ($index = 0; $index -lt @($homeSnapshot.items).Count; $index++) {
            if ($homeSnapshot.items[$index].kind -eq 'Navigation' -and $homeSnapshot.items[$index].target -ceq $prefix) { $position = $index; break }
        }
        Assert-Test -Context $context -Condition ($position -ge 0) -Name ('home exposes prefix ' + $prefix)
        for ($step = 0; $step -lt $position; $step++) { [void]$context.Api.Send($edit, 0x0100, 0x28, 0) }
        $selected = Read-TestSnapshot -Context $context
        $row = $selected.items[$selected.selected]
        Assert-Test -Context $context -Condition ($selected.selected -eq $position -and $row.kind -eq 'Navigation' -and $row.target -ceq $prefix) -Name ('Down selects only navigation for ' + $prefix)
        [void]$context.Api.Send($edit, 0x0100, 0x0D, 0)
        Assert-OwnPrefix ('home ' + $prefix.Trim()) $prefix
    }

    $dpi = $context.Api.GetDpi($context.Main)
    foreach ($tab in @(
        @{ name = 'applications tab'; x = 120; prefix = '@ ' },
        @{ name = 'files tab'; x = 260; prefix = '/ ' },
        @{ name = 'web tab'; x = 340; prefix = '? ' },
        @{ name = 'modules tab'; x = 440; prefix = ';' }
    )) {
        Set-OwnQuery ''
        $x = [int][Math]::Round($tab.x * $dpi / 96)
        $y = [int][Math]::Round(138 * $dpi / 96)
        $point = ($y -shl 16) -bor $x
        [void]$context.Api.Send($context.Main, 0x0201, 1, $point) # owned client WM_LBUTTONDOWN
        Assert-OwnPrefix $tab.name $tab.prefix
    }
    Set-OwnQuery ''
    [void]$context.Api.Send($edit, 0x0100, 9, 0)
    Assert-OwnPrefix 'keyboard Tab opens modules' ';'

    if ($Capture) {
        Set-OwnQuery 'steam'
        $captures.Add((Save-TestWindow -Context $context -Window $context.Main -Path (Join-Path $OutputDirectory 'launcher.png')))
    }
    $beforeFinish = Read-TestSnapshot -Context $context
    Assert-Test -Context $context -Condition ($beforeFinish.fixture_activation_attempts -eq 0) -Name 'no external activation was attempted'
    $native = Complete-TestLauncher -Context $context
    $requiredSmokeChecks = @(
        'all_providers', 'about_menu_entry', 'timer_menu_entry', 'timer_preview_without_start',
        'timer_invalid_duration', 'package_menu_entries', 'applications', 'default_applications',
        'explicit_files', 'install_catalog', 'remove_catalog', 'calculator', 'web_encoding',
        'explicit_command', 'keyboard_selection', 'escape_clears', 'system_actions'
    )
    $smokeNames = @($native.checks | ForEach-Object { $_[0] })
    foreach ($name in $requiredSmokeChecks) {
        Assert-Test -Context $context -Condition ($name -in $smokeNames) -Name ('native smoke retains coverage: ' + $name)
    }
    [void](Write-TestReport -Path $Report -Result ([ordered]@{
        suite = 'native launcher regressions'; passed = $true; theme = $Theme; test_process_id = $context.Pid
        cases = $cases.ToArray(); checks = $context.Checks.ToArray(); native = $native; captures = $captures.ToArray()
        activation_commands_sent = 0; package_operations = 0
    }))
    Write-Output ('Launcher: {0} assertions, {1} cases, report: {2}' -f $context.Checks.Count, $cases.Count, [IO.Path]::GetFullPath($Report))
} catch {
    $failure = $_
    try {
        [void](Write-TestReport -Path $Report -Result ([ordered]@{
            suite = 'native launcher regressions'; passed = $false; theme = $Theme; test_process_id = $context.Pid
            error = $failure.Exception.Message; cases = $cases.ToArray(); checks = $context.Checks.ToArray(); captures = $captures.ToArray()
        }))
        Write-Output ('Launcher: {0} assertions, {1} cases, report: {2}' -f $context.Checks.Count, $cases.Count, [IO.Path]::GetFullPath($Report))
    } catch { } # Preserve the original failed assertion even if writing its report fails.
    throw $failure
} finally {
    Stop-TestLauncher -Context $context
}
