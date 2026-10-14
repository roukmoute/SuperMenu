<#
.SYNOPSIS
Verifies the real launcher and language picker with the checked-in English pack.
No user settings, global input or network download is used: the suite owns its cache.
.EXAMPLE
pwsh -NoProfile -File tests/windows/translations.ps1 -Executable target/release/supermenu.exe
.EXAMPLE
pwsh -NoProfile -File tests/windows/translations.ps1 -Executable target/release/supermenu.exe -Download
The optional -Download starts with an empty cache and exercises the real native
download through the picker. It requires the checked-in pack to be published on
GitHub; the default and CI runs remain fully offline.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [string]$Fixture = (Join-Path $PSScriptRoot '../fixtures/launcher.json'),
    [string]$LanguagePack = (Join-Path $PSScriptRoot '../../locales/en.json'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../../artifacts/test-results'),
    [string]$Report,
    [switch]$Capture,
    [switch]$Download,
    [ValidateRange(30, 300)][int]$TimeoutSeconds = 90
)

. (Join-Path $PSScriptRoot 'support.ps1')

$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (-not $Report) { $Report = Join-Path $OutputDirectory 'translations.json' }
$context = $null
$cacheWorkspace = $null
$captures = [Collections.Generic.List[string]]::new()
$observations = [Collections.Generic.List[object]]::new()
$failure = $null
$packHash = $null
$packWriteTime = $null

try {
    $packPath = [IO.Path]::GetFullPath($LanguagePack)
    if (-not [IO.File]::Exists($packPath)) { throw 'The checked-in English language pack is missing.' }
    $pack = [IO.File]::ReadAllText($packPath) | ConvertFrom-Json -AsHashtable
    if ($pack.schema_version -ne 1 -or $pack.language -cne 'en') { throw 'The test requires an English schema-version 1 language pack.' }
    $cacheWorkspace = New-TestWorkspace
    $cachedPack = Join-Path $cacheWorkspace.Root 'en.json'
    if (-not $Download) {
        Copy-Item -LiteralPath $packPath -Destination $cachedPack
        $packHash = (Get-FileHash -LiteralPath $cachedPack -Algorithm SHA256).Hash
        $packWriteTime = ([IO.FileInfo]::new($cachedPack)).LastWriteTimeUtc
    }
    $context = Start-TestLauncher -Executable $Executable -Fixture $Fixture -Language 'fr' -LanguageDirectory $cacheWorkspace.Root -FirstLanguagePicker -TimeoutSeconds $TimeoutSeconds

    function Select-TranslationLanguage {
        param([string]$Language, [switch]$InitialChoice)
        # Posted so the app can enter its real modal message loop without blocking the harness.
        if (-not $InitialChoice) { $context.Api.Post($context.Main, 0x800C, 0, 0) }
        Wait-TestCondition -Context $context -Message 'the owned language picker appears' -Condition {
            $picker = $context.Api.FindWindow('SuperMenuLanguage')
            return $picker -ne [IntPtr]::Zero -and $context.Api.IsVisible($picker)
        }
        $picker = $context.Api.FindWindow('SuperMenuLanguage')
        if ($InitialChoice) {
            Assert-Test -Context $context -Condition (-not $context.Api.IsVisible($context.Main)) -Name 'the first language choice appears before the launcher is shown'
        }
        $controls = @($context.Api.GetControls($picker))
        $combo = @($controls | Where-Object { $_.Id -eq 401 })
        $continue = @($controls | Where-Object { $_.Id -eq 403 })
        Assert-Test -Context $context -Condition ($combo.Count -eq 1 -and $continue.Count -eq 1) -Name 'the picker exposes its native selection and Continue controls'
        $selectedIndex = if ($Language -ceq 'fr') { 0 } else { 1 }
        $selected = $context.Api.Send([IntPtr]$combo[0].Hwnd, 0x014E, $selectedIndex, 0).ToInt64() # CB_SETCURSEL
        Assert-Test -Context $context -Condition ($selected -eq $selectedIndex) -Name ('the picker selects ' + $Language)
        if ($Capture) {
            $stage = if ($InitialChoice) { 'first-run' } else { $Language }
            $captures.Add((Save-TestWindow -Context $context -Window $picker -Path (Join-Path $OutputDirectory ('translations-language-' + $stage + '.png'))))
        }
        [void]$context.Api.Send([IntPtr]$continue[0].Hwnd, 0x00F5, 0, 0) # BM_CLICK
        Wait-TestCondition -Context $context -TimeoutSeconds 20 -Message ('the picker applies cached language ' + $Language) -Condition {
            if ($context.Api.FindWindow('SuperMenuLanguage') -ne [IntPtr]::Zero) { return $false }
            return (Read-TestSnapshot -Context $context).language -ceq $Language
        }
        if ($InitialChoice) {
            Wait-TestCondition -Context $context -Message 'the launcher becomes visible after choosing its first language' -Condition { return $context.Api.IsVisible($context.Main) }
            Assert-Test -Context $context -Condition ($context.Api.IsVisible($context.Main)) -Name 'the launcher appears after the first language choice is applied'
        }
        if ($null -ne $packHash) {
            Assert-Test -Context $context -Condition ((Get-FileHash -LiteralPath $cachedPack -Algorithm SHA256).Hash -ceq $packHash) -Name 'changing language preserves the cached pack contents'
            Assert-Test -Context $context -Condition (([IO.FileInfo]::new($cachedPack)).LastWriteTimeUtc -eq $packWriteTime) -Name 'the valid cached translation is reused without being rewritten'
        }
    }

    if ($Download) {
        Assert-Test -Context $context -Condition (-not [IO.File]::Exists($cachedPack)) -Name 'the optional network test begins with an empty translation cache'
    }
    Select-TranslationLanguage 'en' -InitialChoice
    if ($Download) {
        Assert-Test -Context $context -Condition ([IO.File]::Exists($cachedPack)) -Name 'the native picker downloads the chosen English pack'
        $packHash = (Get-FileHash -LiteralPath $cachedPack -Algorithm SHA256).Hash
        $expectedPackHash = (Get-FileHash -LiteralPath $packPath -Algorithm SHA256).Hash
        Assert-Test -Context $context -Condition ($packHash -ceq $expectedPackHash) -Name 'the downloaded pack exactly matches the published repository pack'
        $packWriteTime = ([IO.FileInfo]::new($cachedPack)).LastWriteTimeUtc
    }
    [void]$context.Api.Send($context.Main, 0x8002, 0, 0)
    $editControls = @($context.Api.GetControls($context.Main) | Where-Object { $_.Class -eq 'EDIT' })
    Assert-Test -Context $context -Condition ($editControls.Count -eq 1) -Name 'one native search control exists'
    $edit = [IntPtr]$editControls[0].Hwnd

    function Set-TranslationQuery([string]$Text) {
        [void]$context.Api.SendText($edit, 0x000C, 0, '')
        $context.Api.TypeCharacters($edit, $Text)
        return Read-TestSnapshot -Context $context
    }

    function Assert-TranslationHome([string]$Language) {
        $snapshot = Set-TranslationQuery ''
        $expected = if ($Language -ceq 'en') {
            @{ 'nav:@ ' = 'Applications'; 'nav:+ ' = 'Install'; 'nav:- ' = 'Uninstall'; 'nav:minuteur ' = 'Timer'; 'nav:about' = 'About' }
        } else {
            @{ 'nav:@ ' = 'Applications'; 'nav:+ ' = 'Installer'; 'nav:- ' = 'Désinstaller'; 'nav:minuteur ' = 'Minuteur'; 'nav:about' = 'À propos' }
        }
        Assert-Test -Context $context -Condition ($snapshot.language -ceq $Language) -Name ('active language is ' + $Language)
        foreach ($id in $expected.Keys) {
            $rows = @($snapshot.items | Where-Object { $_.id -ceq $id })
            Assert-Test -Context $context -Condition ($rows.Count -eq 1 -and $rows[0].title -ceq $expected[$id]) -Name ($Language + ': translated navigation ' + $id)
        }
        $observations.Add([ordered]@{ language = $Language; query = $snapshot.query; items = @($snapshot.items) })
    }

    function Assert-TranslationAbout([string]$Language) {
        [void]$context.Api.Send($context.Main, 0x8009, 0, 0)
        Wait-TestCondition -Context $context -Message ('the ' + $Language + ' About window is visible') -Condition {
            $window = $context.Api.FindWindow('SuperMenuAboutWindow')
            return $window -ne [IntPtr]::Zero -and $context.Api.IsVisible($window)
        }
        $window = $context.Api.FindWindow('SuperMenuAboutWindow')
        Wait-TestCondition -Context $context -TimeoutSeconds 20 -Message ('the ' + $Language + ' system snapshot is collected') -Condition {
            $snapshot = Read-TestSnapshot -Context $context
            return $snapshot.about_loaded -and -not $snapshot.about_collecting
        }
        $controls = @($context.Api.GetControls($window))
        $texts = @($controls | Where-Object { $_.Class -eq 'STATIC' } | ForEach-Object { $_.Text })
        $caption = if ($Language -ceq 'en') { 'Super Menu · About' } else { 'Super Menu · À propos' }
        $labels = if ($Language -ceq 'en') {
            @('Hardware', 'Windows', 'Activity', 'Processor', 'Graphics', 'Display', 'Memory', 'System drive', 'Edition', 'Architecture', 'Uptime', 'Windows installed on', 'Super Menu executable', '7 applications', '3 files and folders', '10 entries in the local index')
        } else {
            @('Matériel', 'Windows', 'Activité', 'Processeur', 'Graphiques', 'Écran', 'Mémoire', 'Disque système', 'Édition', 'Architecture', 'Allumé depuis', 'Windows installé le', 'Exécutable Super Menu', '7 applications', '3 fichiers et dossiers', "10 entrées dans l’index local")
        }
        Assert-Test -Context $context -Condition ($context.Api.ReadText($window) -ceq $caption) -Name ($Language + ': About caption is translated')
        foreach ($label in $labels) {
            Assert-Test -Context $context -Condition ($texts -ccontains $label) -Name ($Language + ': About label ' + $label)
        }
        $close = @($controls | Where-Object { $_.Id -eq 3102 })
        $closeText = if ($Language -ceq 'en') { 'Close' } else { 'Fermer' }
        Assert-Test -Context $context -Condition ($close.Count -eq 1 -and $close[0].Text -ceq $closeText) -Name ($Language + ': Close button is translated')
        $binarySize = ([IO.FileInfo]::new([IO.Path]::GetFullPath($Executable))).Length
        $byteUnit = if ($Language -ceq 'en') { 'bytes' } else { 'octets' }
        Assert-Test -Context $context -Condition (@($texts | Where-Object { $_ -match ('\(' + $binarySize + ' ' + $byteUnit + '\)$') }).Count -eq 1) -Name ($Language + ': executable size uses the translated byte unit')
        $observations.Add([ordered]@{ language = $Language; about_caption = $caption; about_texts = $texts })
        if ($Capture) { $captures.Add((Save-TestWindow -Context $context -Window $window -Path (Join-Path $OutputDirectory ('translations-about-' + $Language + '.png')))) }
        [void]$context.Api.Send([IntPtr]$close[0].Hwnd, 0x00F5, 0, 0)
        Wait-TestCondition -Context $context -Message 'the owned About window closes' -Condition { return -not $context.Api.IsVisible($window) }
    }

    Assert-TranslationHome 'en'
    $apps = Set-TranslationQuery '@ steam'
    Assert-Test -Context $context -Condition ($apps.items.Count -eq 2 -and $apps.items[0].title -ceq 'Steam') -Name 'translated search preserves application names and results'
    Assert-Test -Context $context -Condition (@($apps.items | Where-Object { $_.subtitle -ceq 'Windows application' }).Count -eq 2) -Name 'application subtitles are translated'
    $install = Set-TranslationQuery '+ obsidian'
    Assert-Test -Context $context -Condition ($install.items.Count -eq 1 -and $install.items[0].id -ceq 'install:Obsidian.Obsidian' -and $install.items[0].title -ceq 'Obsidian' -and $install.items[0].subtitle -match '^Install\b') -Name 'the installation action is translated without changing its package identity'
    $uninstall = Set-TranslationQuery '- steam'
    Assert-Test -Context $context -Condition ($uninstall.items.Count -eq 1 -and $uninstall.items[0].id -ceq 'uninstall:id:Valve.Steam' -and $uninstall.items[0].title -ceq 'Steam' -and $uninstall.items[0].subtitle -match '^Uninstall\b') -Name 'the removal action is translated without activating it'
    $timer = Set-TranslationQuery 'timer 5s Tea'
    Assert-Test -Context $context -Condition ($timer.items.Count -eq 1 -and $timer.items[0].kind -ceq 'Timer' -and $timer.items[0].title -ceq 'Start · 00:05') -Name 'the English timer alias exposes a translated preview'
    Assert-Test -Context $context -Condition ($timer.timer_count -eq 0 -and $timer.items[0].subtitle -match 'Enter to start') -Name 'translation tests do not start a countdown'
    Assert-TranslationAbout 'en'

    Select-TranslationLanguage 'fr'
    Assert-TranslationHome 'fr'
    Assert-TranslationAbout 'fr'
    Select-TranslationLanguage 'en'
    Assert-TranslationHome 'en'
    Assert-TranslationAbout 'en'

    $beforeFinish = Read-TestSnapshot -Context $context
    Assert-Test -Context $context -Condition ($beforeFinish.fixture_activation_attempts -eq 0) -Name 'no external program or package action was activated'
    Assert-Test -Context $context -Condition ($beforeFinish.timer_count -eq 0) -Name 'no timer was started'
    Assert-Test -Context $context -Condition (@(Get-ChildItem -LiteralPath $cacheWorkspace.Root -File | Where-Object { $_.Name -ne '.supermenu-test-workspace' }).Count -eq 1) -Name 'only the selected English pack exists in the owned cache'
    $native = Complete-TestLauncher -Context $context
    [void](Write-TestReport -Path $Report -Result ([ordered]@{
        suite = 'native translations'; passed = $true; test_process_id = $context.Pid
        download_requested = [bool]$Download
        checks = @($context.Checks); observations = @($observations); captures = @($captures); native_smoke = $native
    }))
    Write-Host ('Translations: ' + $context.Checks.Count + ' assertions, report: ' + [IO.Path]::GetFullPath($Report))
} catch {
    $failure = $_
    [void](Write-TestReport -Path $Report -Result ([ordered]@{
        suite = 'native translations'; passed = $false; error = $_.Exception.Message
        download_requested = [bool]$Download
        test_process_id = $(if ($null -ne $context) { $context.Pid } else { $null })
        checks = $(if ($null -ne $context) { @($context.Checks) } else { @() })
        observations = @($observations); captures = @($captures)
    }))
} finally {
    try { if ($null -ne $context) { Stop-TestLauncher -Context $context } }
    finally { if ($null -ne $cacheWorkspace) { Remove-TestWorkspace -Workspace $cacheWorkspace } }
}
if ($null -ne $failure) { throw $failure }
