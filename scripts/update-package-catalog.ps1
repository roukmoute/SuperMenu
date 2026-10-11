param(
    [switch]$Check,
    [switch]$Update,
    [string]$Revision = ''
)

# Maintainer-only generation. Super Menu never reads/downloads this JSON at runtime.
$ErrorActionPreference = 'Stop'
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$catalogDirectory = Join-Path $repositoryRoot 'catalog'
$snapshotPath = Join-Path $catalogDirectory 'winutil-applications.json'
$metadataPath = Join-Path $catalogDirectory 'winutil-source.json'
$categories = @{
    'Browsers' = 'Navigateur'; 'Communications' = 'Messagerie'; 'Development' = 'Développement'
    'Document' = 'Documents'; 'Games' = 'Jeux'; 'Microsoft Tools' = 'Outils Microsoft'
    'Multimedia Tools' = 'Multimédia'; 'Pro Tools' = 'Outils professionnels'
    'Selfhosted Tools' = 'Auto-hébergement'; 'Utilities' = 'Utilitaires Windows'
}
if ($Update) {
    if ($Check -or $Revision -notmatch '^[a-fA-F0-9]{40}$') {
        throw 'Use -Update -Revision <full commit SHA>, separately from -Check.'
    }
    $Revision = $Revision.ToLowerInvariant()
    $url = "https://raw.githubusercontent.com/ChrisTitusTech/winutil/$Revision/config/applications.json"
    # Validate downloaded data before replacing the committed snapshot.
    $response = Invoke-WebRequest -Uri $url
    $downloaded = $response.Content
    if ($downloaded -is [byte[]]) { $downloaded = [Text.Encoding]::UTF8.GetString($downloaded) }
    $candidate = $downloaded | ConvertFrom-Json -AsHashtable
    if ($candidate.Count -eq 0) { throw 'Empty upstream catalogue.' }
    $candidateIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($entry in $candidate.Values) {
        if ($entry.winget -notmatch '^(msstore:[A-Z0-9]{12}|[A-Za-z0-9_+\-]+(?:\.[A-Za-z0-9_+\-]+)+)$' -or
            [string]::IsNullOrWhiteSpace($entry.content) -or $entry.content -match '[\x00-\x1F\x7F]' -or
            -not $categories.ContainsKey($entry.category) -or -not $candidateIds.Add($entry.winget)) {
            throw 'Unsupported upstream package format; review the new snapshot manually.'
        }
    }
    [IO.File]::WriteAllText($snapshotPath, $downloaded, [Text.UTF8Encoding]::new($false))
    [ordered]@{
        commit = $Revision
        source_url = "https://github.com/ChrisTitusTech/winutil/blob/$Revision/config/applications.json"
        source_sha256 = (Get-FileHash -LiteralPath $snapshotPath -Algorithm SHA256).Hash
        application_count = $candidate.Count
    } | ConvertTo-Json | Set-Content -LiteralPath $metadataPath -Encoding utf8NoBOM
}
elseif ($Revision) { throw '-Revision requires -Update.' }

$metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $snapshotPath -Algorithm SHA256).Hash -ne $metadata.source_sha256) {
    throw 'Snapshot hash differs from the recorded source.'
}
$upstream = Get-Content -LiteralPath $snapshotPath -Raw | ConvertFrom-Json -AsHashtable
if ($upstream.Count -ne $metadata.application_count) { throw 'Snapshot entry count differs from provenance.' }
$local = @(Get-Content -LiteralPath (Join-Path $catalogDirectory 'packages.local.json') -Raw | ConvertFrom-Json)
$aliases = Get-Content -LiteralPath (Join-Path $catalogDirectory 'aliases.json') -Raw | ConvertFrom-Json -AsHashtable
$packages = [Collections.Generic.List[object]]::new()
$ids = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($entry in $local) {
    if (-not $ids.Add($entry.id)) { throw "Duplicate local ID: $($entry.id)" }
    $packages.Add([ordered]@{ id=$entry.id; name=$entry.name; category=$entry.category; aliases=@($entry.aliases); source='Winget'; install=$entry.install; removal=$entry.removal })
}
# Stable ordinal ID order, while preserving the reviewed local catalogue order.
$upstreamKeys = [string[]]@($upstream.Keys)
[Array]::Sort($upstreamKeys, [StringComparer]::Ordinal)
$upstreamIds = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($key in $upstreamKeys) {
    $entry = $upstream[$key]
    $store = $entry.winget.StartsWith('msstore:', [StringComparison]::Ordinal)
    $id = if ($store) { $entry.winget.Substring(8) } else { $entry.winget }
    if (-not $upstreamIds.Add($id)) { throw "Duplicate upstream ID: $id" }
    if (($store -and $id -notmatch '^[A-Z0-9]{12}$') -or
        (-not $store -and $id -notmatch '^[A-Za-z0-9_+\-]+(?:\.[A-Za-z0-9_+\-]+)+$')) {
        throw "Unsupported package ID: $($entry.winget)"
    }
    if (-not $categories.ContainsKey($entry.category)) { throw "Unmapped category: $($entry.category)" }
    if ($ids.Add($id)) {
        $name = [regex]::Replace($entry.content, '\s+\([^()]*\)$', '').Trim()
        $packages.Add([ordered]@{ id=$id; name=$name; category=$categories[$entry.category]; aliases=@(); source=$(if ($store) {'MicrosoftStore'} else {'Winget'}); install=$true; removal=$false })
    }
    $package = $packages | Where-Object { $_.id -ieq $id } | Select-Object -First 1
    if ($package.source -ne $(if ($store) {'MicrosoftStore'} else {'Winget'})) { throw "Source conflict: $id" }
    if ($package.name -ine $entry.content -and $package.aliases -inotcontains $entry.content) {
        $package.aliases += $entry.content
    }
}
foreach ($id in $aliases.Keys) {
    if (-not $ids.Contains($id)) { throw "Alias references an unknown ID: $id" }
}

function ConvertTo-RustLiteral([string]$Value) {
    if ($Value -match '[\x00-\x1F\x7F]') { throw 'Control characters in catalogue string.' }
    return '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"'
}
$lines = [Collections.Generic.List[string]]::new()
$lines.Add('// Generated by scripts/update-package-catalog.ps1; do not edit by hand.')
$lines.Add("// WinUtil revision: $($metadata.commit). See THIRD_PARTY_NOTICES.md.")
$lines.Add('#[rustfmt::skip]')
$lines.Add('pub const PACKAGES: &[Package] = &[')
foreach ($package in $packages) {
    $typingAliases = @($package.aliases) + @($aliases[$package.id]) | Where-Object { $_ -and $_ -ine $package.name } | Select-Object -Unique
    $aliasLiterals = @($typingAliases | ForEach-Object { ConvertTo-RustLiteral $_ }) -join ', '
    $install = $package.install.ToString().ToLowerInvariant()
    $removal = $package.removal.ToString().ToLowerInvariant()
    $lines.Add(('    package!({0}, {1}, {2}, [{3}], {4}, {5}, {6}),' -f (ConvertTo-RustLiteral $package.id), (ConvertTo-RustLiteral $package.name), (ConvertTo-RustLiteral $package.category), $aliasLiterals, $package.source, $install, $removal))
}
$lines.Add('];')
$generated = ($lines -join "`r`n") + "`r`n"
$outputPath = Join-Path $repositoryRoot 'src\package_catalog.rs'
if ($Check) {
    if (-not (Test-Path -LiteralPath $outputPath) -or [IO.File]::ReadAllText($outputPath).Replace("`r`n", "`n") -cne $generated.Replace("`r`n", "`n")) {
        throw 'Generated package catalogue is stale. Run scripts/update-package-catalog.ps1.'
    }
}
else { [IO.File]::WriteAllText($outputPath, $generated, [Text.UTF8Encoding]::new($false)) }
Write-Output "Catalogue: $($packages.Count) IDs, $($upstream.Count) WinUtil entries, $(@($packages | Where-Object { $_.install }).Count) installation choices."
