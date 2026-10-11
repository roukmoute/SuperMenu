param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [string]$OutputDirectory = '',
    [string]$Version = '',
    [string]$NsisDirectory = '',
    [string]$TestNamespace = '',
    [string]$TestInstallDirectory = '',
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'The Windows setup must be built on Windows.' }
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$Executable = [IO.Path]::GetFullPath($Executable)
if (-not [IO.File]::Exists($Executable)) { throw "Executable not found: $Executable" }
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot 'artifacts\setup'
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null

if ([string]::IsNullOrWhiteSpace($Version)) {
    $cargo = [IO.File]::ReadAllText((Join-Path $repositoryRoot 'Cargo.toml'))
    $versionMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $versionMatch.Success) { throw 'Version missing from Cargo.toml.' }
    $Version = $versionMatch.Groups[1].Value
}
$versionMatch = [regex]::Match($Version, '^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$')
if (-not $versionMatch.Success) { throw 'Version must be a semantic version.' }
$productVersion = ($versionMatch.Groups[1..3] | ForEach-Object { $_.Value }) -join '.'
$productVersion += '.0'

# Reject a wrong payload early, without executing it. NSIS remains only a build tool.
$stream = [IO.File]::OpenRead($Executable)
$reader = [IO.BinaryReader]::new($stream)
try {
    if ($reader.ReadUInt16() -ne 0x5A4D) { throw 'The payload is not a Windows executable.' }
    $stream.Position = 0x3C
    $peOffset = $reader.ReadUInt32()
    if ($peOffset -gt $stream.Length - 6) { throw 'Invalid PE header.' }
    $stream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
        throw 'The payload must be a Windows x64 executable.'
    }
} finally {
    $reader.Dispose()
    $stream.Dispose()
}

$installId = 'SuperMenu'
$shortcutName = 'Super Menu'
$displayName = 'Super Menu'
$defaultInstallDirectory = '$LOCALAPPDATA\Programs\Super Menu'
$fixture = -not [string]::IsNullOrWhiteSpace($TestNamespace)
if ($fixture) {
    $namespaceGuid = [guid]::Empty
    if (-not [guid]::TryParse($TestNamespace, [ref]$namespaceGuid) -or $namespaceGuid -eq [guid]::Empty) {
        throw 'TestNamespace must be a nonempty GUID.'
    }
    $TestNamespace = $namespaceGuid.ToString('D')
    $installId = 'SuperMenuSetupFixture-' + $TestNamespace
    $shortcutName = $installId
    $displayName = 'Super Menu (fixture ' + $TestNamespace + ')'
    if ([string]::IsNullOrWhiteSpace($TestInstallDirectory)) {
        $TestInstallDirectory = Join-Path $OutputDirectory ('installed-' + $TestNamespace)
    }
    if (-not [IO.Path]::IsPathRooted($TestInstallDirectory)) { throw 'TestInstallDirectory must be absolute.' }
    $defaultInstallDirectory = [IO.Path]::GetFullPath($TestInstallDirectory).TrimEnd([char[]]'\/')
    if ($defaultInstallDirectory -eq [IO.Path]::GetPathRoot($defaultInstallDirectory).TrimEnd([char[]]'\/')) {
        throw 'A filesystem root cannot be used for a setup fixture.'
    }
} elseif (-not [string]::IsNullOrWhiteSpace($TestInstallDirectory)) {
    throw 'TestInstallDirectory requires TestNamespace.'
}

$nsisVersion = '3.13'
$nsisHash = 'BA63DFFC4410EE89193E1CB5A41989991BD77C61068DA17E3156D136B7B0B3D8'
if ([string]::IsNullOrWhiteSpace($NsisDirectory)) {
    $cacheDirectory = Join-Path $OutputDirectory '.nsis-tools'
    $NsisDirectory = Join-Path $cacheDirectory ('nsis-' + $nsisVersion)
    $compiler = Join-Path $NsisDirectory 'makensis.exe'
    if (-not [IO.File]::Exists($compiler)) {
        if ($Offline) { throw 'NSIS is not cached. Supply NsisDirectory or allow the pinned tool download.' }
        [IO.Directory]::CreateDirectory($cacheDirectory) | Out-Null
        $archivePath = Join-Path $cacheDirectory ('nsis-' + $nsisVersion + '.zip')
        if (-not [IO.File]::Exists($archivePath) -or (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ne $nsisHash) {
            $archiveUrl = 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip'
            Invoke-WebRequest -Uri $archiveUrl -UserAgent 'Wget/1.21.3' -OutFile $archivePath -UseBasicParsing
        }
        if ((Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash -ne $nsisHash) {
            throw 'The pinned NSIS archive failed SHA-256 verification.'
        }
        Expand-Archive -LiteralPath $archivePath -DestinationPath $cacheDirectory -Force
    }
}
$NsisDirectory = [IO.Path]::GetFullPath($NsisDirectory)
$compiler = Join-Path $NsisDirectory 'makensis.exe'
if (-not [IO.File]::Exists($compiler)) { throw "NSIS compiler not found: $compiler" }
$compilerVersion = (& $compiler '/VERSION' | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $compilerVersion -ne ('v' + $nsisVersion)) {
    throw 'This build requires the pinned portable NSIS v3.13 compiler.'
}

function New-BrandIcon {
    param([string]$Path)
    $images = @()
    foreach ($side in @(16, 32)) {
        $imageStream = [IO.MemoryStream]::new()
        $writer = [IO.BinaryWriter]::new($imageStream)
        try {
            $writer.Write([uint32]40)
            $writer.Write([int32]$side)
            $writer.Write([int32]($side * 2))
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]0)
            $writer.Write([uint32]($side * $side * 4))
            foreach ($unused in 1..4) { $writer.Write([uint32]0) }
            $margin = [int]($side / 16)
            $cell = [int](($side - 4 * $margin) / 3)
            $stride = [int]([Math]::Ceiling($side / 32.0) * 4)
            $mask = [byte[]]::new($stride * $side)
            for ($index = 0; $index -lt $mask.Length; $index++) { $mask[$index] = 255 }
            for ($row = $side - 1; $row -ge 0; $row--) {
                for ($column = 0; $column -lt $side; $column++) {
                    $occupiedX = $false
                    $occupiedY = $false
                    for ($block = 0; $block -lt 3; $block++) {
                        $start = $margin + $block * ($cell + $margin)
                        if ($column -ge $start -and $column -lt $start + $cell) { $occupiedX = $true }
                        if ($row -ge $start -and $row -lt $start + $cell) { $occupiedY = $true }
                    }
                    if ($occupiedX -and $occupiedY) {
                        $writer.Write([byte]236)
                        $writer.Write([byte]98)
                        $writer.Write([byte]138)
                        $writer.Write([byte]255)
                        $maskIndex = ($side - $row - 1) * $stride + [int][Math]::Floor($column / 8.0)
                        $bit = 128 -shr ($column % 8)
                        $mask[$maskIndex] = [byte]($mask[$maskIndex] -band (255 -bxor $bit))
                    } else {
                        $writer.Write([uint32]0)
                    }
                }
            }
            $writer.Write($mask)
            $writer.Flush()
            $images += ,$imageStream.ToArray()
        } finally {
            $writer.Dispose()
            $imageStream.Dispose()
        }
    }
    $stream = [IO.File]::Create($Path)
    $writer = [IO.BinaryWriter]::new($stream)
    try {
        $writer.Write([uint16]0)
        $writer.Write([uint16]1)
        $writer.Write([uint16]2)
        $offset = 6 + 2 * 16
        for ($index = 0; $index -lt 2; $index++) {
            $side = @(16, 32)[$index]
            $writer.Write([byte]$side)
            $writer.Write([byte]$side)
            $writer.Write([byte]0)
            $writer.Write([byte]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]32)
            $writer.Write([uint32]$images[$index].Length)
            $writer.Write([uint32]$offset)
            $offset += $images[$index].Length
        }
        foreach ($image in $images) { $writer.Write([byte[]]$image) }
    } finally {
        $writer.Dispose()
        $stream.Dispose()
    }
}

function ConvertTo-NsisLiteral {
    param([string]$Value)
    if ($Value.Contains('"') -or $Value.Contains("`r") -or $Value.Contains("`n")) {
        throw 'NSIS definition contains invalid quotation marks or line breaks.'
    }
    $Value.Replace('$', '$$')
}

$assetDirectory = Join-Path $OutputDirectory '.setup-assets'
[IO.Directory]::CreateDirectory($assetDirectory) | Out-Null
$iconPath = Join-Path $assetDirectory 'brand.ico'
New-BrandIcon -Path $iconPath
$suffix = if ($fixture) { '-fixture-' + $TestNamespace } else { '' }
$setupPath = Join-Path $OutputDirectory ('SuperMenu-Setup-x64' + $suffix + '.exe')
$licensePath = Join-Path $repositoryRoot 'LICENSE'
$noticesPath = Join-Path $repositoryRoot 'THIRD_PARTY_NOTICES.md'
$sizeKb = [int][Math]::Ceiling(((Get-Item -LiteralPath $Executable).Length + (Get-Item -LiteralPath $licensePath).Length + (Get-Item -LiteralPath $noticesPath).Length + (Get-Item -LiteralPath $iconPath).Length + 64KB) / 1KB)
$definitions = @{
    OUTPUT_EXE = $setupPath
    SOURCE_EXE = $Executable
    LICENSE_FILE = $licensePath
    NOTICES_FILE = $noticesPath
    BRAND_ICON = $iconPath
    APP_VERSION = $Version
    PRODUCT_VERSION = $productVersion
    INSTALL_ID = $installId
    SHORTCUT_NAME = $shortcutName
    DISPLAY_NAME = $displayName
    INSTALL_SIZE_KB = [string]$sizeKb
}
$compilerArguments = @('/V2', '/WX', '/INPUTCHARSET', 'UTF8')
foreach ($entry in $definitions.GetEnumerator()) {
    $compilerArguments += '/D' + $entry.Key + '=' + (ConvertTo-NsisLiteral $entry.Value)
}
$compilerArguments += '/DDEFAULT_INSTALL_DIR=' + $(if ($fixture) { ConvertTo-NsisLiteral $defaultInstallDirectory } else { $defaultInstallDirectory })
$compilerArguments += Join-Path $repositoryRoot 'packaging\windows\SuperMenu.nsi'
& $compiler @compilerArguments
if ($LASTEXITCODE -ne 0) { throw "NSIS compilation failed with exit code $LASTEXITCODE." }
if (-not [IO.File]::Exists($setupPath)) { throw 'NSIS did not create the setup executable.' }
$manifest = [ordered]@{
    version = $Version
    installer = [IO.Path]::GetFileName($setupPath)
    installer_bytes = (Get-Item -LiteralPath $setupPath).Length
    installer_sha256 = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash
    payload_bytes = (Get-Item -LiteralPath $Executable).Length
    payload_sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    nsis_version = $nsisVersion
    nsis_archive_sha256 = $nsisHash
    test_namespace = $(if ($fixture) { $TestNamespace } else { $null })
}
$manifestPath = [IO.Path]::ChangeExtension($setupPath, '.json')
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 3), [Text.UTF8Encoding]::new($false))
Write-Host ('Setup: {0} ({1:N0} bytes)' -f $setupPath, $manifest.installer_bytes)
[pscustomobject]@{ Installer = $setupPath; Manifest = $manifestPath; Version = $Version; Bytes = $manifest.installer_bytes; TestNamespace = $TestNamespace; NsisDirectory = $NsisDirectory }
