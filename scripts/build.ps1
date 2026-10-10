param(
    [switch]$SkipChecks,
    [switch]$Archive,
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Super Menu se compile uniquement sous Windows.' }
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot 'artifacts\SuperMenu'
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$previousRustFlags = $env:RUSTFLAGS

function Invoke-Cargo {
    param([string[]]$CargoArguments)
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) { throw "cargo a échoué avec le code $LASTEXITCODE." }
}

Push-Location -LiteralPath $repositoryRoot
try {
    # Embed the MSVC C runtime so the shipped executable needs no runtime installer.
    $env:RUSTFLAGS = ($previousRustFlags + ' -C target-feature=+crt-static').Trim()
    if (-not $SkipChecks) {
        Invoke-Cargo -CargoArguments @('fmt', '--check')
        Invoke-Cargo -CargoArguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
        Invoke-Cargo -CargoArguments @('test', '--locked')
    }
    Invoke-Cargo -CargoArguments @('build', '--release', '--locked')
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $benchmarkPath = Join-Path $OutputDirectory 'benchmark.txt'
    Invoke-Cargo -CargoArguments @('run', '--release', '--locked', '--', '--benchmark', '--report', $benchmarkPath)
    if (-not (Test-Path -LiteralPath $benchmarkPath) -or (Get-Item -LiteralPath $benchmarkPath).Length -eq 0) {
        throw 'Le benchmark n''a pas produit son rapport.'
    }
    $metadataText = & cargo metadata --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw 'Impossible de lire les métadonnées Cargo.' }
    $metadata = $metadataText | ConvertFrom-Json
    $binaryTarget = @($metadata.packages | ForEach-Object { $_.targets } | Where-Object { $_.kind -contains 'bin' })[0]
    if ($null -eq $binaryTarget) { throw 'Aucune cible exécutable Cargo trouvée.' }
    $binaryPath = Join-Path $metadata.target_directory ("release\" + $binaryTarget.name + '.exe')
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $OutputDirectory ($binaryTarget.name + '.exe')) -Force
    foreach ($name in @('README.md', 'LICENSE', 'settings.example.json')) {
        Copy-Item -LiteralPath (Join-Path $repositoryRoot $name) -Destination (Join-Path $OutputDirectory $name) -Force
    }
    $docsDirectory = Join-Path $OutputDirectory 'docs'
    New-Item -ItemType Directory -Path $docsDirectory -Force | Out-Null
    foreach ($image in @('screenshot.png', 'install.png', 'launch.png', 'remove.png', 'timer.png')) {
        Copy-Item -LiteralPath (Join-Path $repositoryRoot (Join-Path 'docs' $image)) -Destination (Join-Path $docsDirectory $image) -Force
    }
    $size = (Get-Item -LiteralPath $binaryPath).Length / 1MB
    Write-Host ('Exécutable : {0:N2} MiB' -f $size)
    Write-Host "Distribution : $OutputDirectory"
    if ($Archive) {
        $archivePath = Join-Path ([IO.Path]::GetDirectoryName($OutputDirectory)) 'SuperMenu-windows-x64.zip'
        Compress-Archive -Path (Join-Path $OutputDirectory '*') -DestinationPath $archivePath -Force
        Write-Host "Archive : $archivePath"
    }
}
finally {
    $env:RUSTFLAGS = $previousRustFlags
    Pop-Location
}
