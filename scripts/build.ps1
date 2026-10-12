param(
    [switch]$SkipChecks,
    [switch]$Archive,
    [switch]$Setup,
    [switch]$Nightly,
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

# Optional smaller build: nightly Rust rebuilds the standard library for size and aborts
# panics without formatting a message. A windows-subsystem executable has no console to
# print one anyway, so only the panic and backtrace machinery disappears from the binary.
# The toolchain is selected through PATH so that cargo, rustc and the sysroot all match.
function Build-NightlyRelease {
    param([string]$TargetDirectory, [string]$BinaryName)
    $nightlyCargo = & rustup which --toolchain nightly cargo 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($nightlyCargo)) {
        throw 'La construction nightly nécessite rustup et le toolchain nightly : rustup toolchain install nightly --component rust-src'
    }
    $installed = @(& rustup component list --toolchain nightly --installed)
    if (-not ($installed | Where-Object { $_ -match '^rust-src' })) {
        throw 'Le composant rust-src du toolchain nightly est requis : rustup component add rust-src --toolchain nightly'
    }
    $previousPath = $env:PATH
    $previousFlags = $env:RUSTFLAGS
    try {
        $env:PATH = (Split-Path -Parent $nightlyCargo) + [IO.Path]::PathSeparator + $env:PATH
        $env:RUSTFLAGS = ($previousFlags + ' -Zunstable-options -Cpanic=immediate-abort').Trim()
        Invoke-Cargo -CargoArguments @('build', '--release', '--locked', '--target', 'x86_64-pc-windows-msvc', '-Zbuild-std=std,panic_abort', '-Zbuild-std-features=optimize_for_size')
    }
    finally {
        $env:PATH = $previousPath
        $env:RUSTFLAGS = $previousFlags
    }
    return Join-Path $TargetDirectory ('x86_64-pc-windows-msvc\release\' + $BinaryName + '.exe')
}

Push-Location -LiteralPath $repositoryRoot
try {
    # Embed the MSVC C runtime so the shipped executable needs no runtime installer.
    $env:RUSTFLAGS = ($previousRustFlags + ' -C target-feature=+crt-static').Trim()
    if (-not $SkipChecks) {
        & (Join-Path $PSScriptRoot 'update-package-catalog.ps1') -Check
        Invoke-Cargo -CargoArguments @('fmt', '--check')
        Invoke-Cargo -CargoArguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
        Invoke-Cargo -CargoArguments @('test', '--locked')
    }
    $metadataText = & cargo metadata --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw 'Impossible de lire les métadonnées Cargo.' }
    $metadata = $metadataText | ConvertFrom-Json
    $binaryTarget = @($metadata.packages | ForEach-Object { $_.targets } | Where-Object { $_.kind -contains 'bin' })[0]
    if ($null -eq $binaryTarget) { throw 'Aucune cible exécutable Cargo trouvée.' }
    if ($Nightly) {
        $binaryPath = Build-NightlyRelease -TargetDirectory $metadata.target_directory -BinaryName $binaryTarget.name
    } else {
        Invoke-Cargo -CargoArguments @('build', '--release', '--locked')
        $binaryPath = Join-Path $metadata.target_directory ("release\" + $binaryTarget.name + '.exe')
    }
    if (-not (Test-Path -LiteralPath $binaryPath)) { throw "Exécutable absent après la compilation : $binaryPath" }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $benchmarkPath = Join-Path $OutputDirectory 'benchmark.txt'
    # The benchmark runs the exact executable that is distributed, whichever toolchain built it.
    # Start-Process waits for this windows-subsystem program, unlike the call operator.
    $benchmark = Start-Process -FilePath $binaryPath -ArgumentList @('--benchmark', '--report', $benchmarkPath) -NoNewWindow -Wait -PassThru
    if ($benchmark.ExitCode -ne 0) { throw "Le benchmark a échoué avec le code $($benchmark.ExitCode)." }
    if (-not (Test-Path -LiteralPath $benchmarkPath) -or (Get-Item -LiteralPath $benchmarkPath).Length -eq 0) {
        throw 'Le benchmark n''a pas produit son rapport.'
    }
    if (-not $SkipChecks) {
        & (Join-Path $PSScriptRoot 'test.ps1') -Executable $binaryPath -SkipRustChecks
        if (-not $?) { throw 'Les tests du menu Windows ont échoué.' }
    }
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $OutputDirectory ($binaryTarget.name + '.exe')) -Force
    foreach ($name in @('README.md', 'TESTING.md', 'PERFORMANCE.md', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'settings.example.json')) {
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
    if ($Setup) {
        $setupDirectory = [IO.Path]::GetDirectoryName($OutputDirectory)
        $setupBuild = @(& (Join-Path $PSScriptRoot 'build-setup.ps1') -Executable $binaryPath -OutputDirectory $setupDirectory)[-1]
        if (-not $?) { throw 'La compilation de l''installateur a échoué.' }
        if (-not $SkipChecks) {
            & (Join-Path $repositoryRoot 'tests/windows/setup.ps1') -Executable $binaryPath -NsisDirectory $setupBuild.NsisDirectory
            if (-not $?) { throw 'Les tests de l''installateur ont échoué.' }
        }
    }
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
