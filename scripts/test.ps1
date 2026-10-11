#requires -Version 7.0
param(
    [string]$Executable = '',
    [switch]$SkipRustChecks,
    [switch]$Capture,
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Les tests natifs nécessitent Windows.' }
$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repositoryRoot 'artifacts\test-results'
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$previousRustFlags = $env:RUSTFLAGS

function Invoke-TestCargo([string[]]$CargoArguments) {
    & cargo @CargoArguments
    if ($LASTEXITCODE -ne 0) { throw "cargo a échoué avec le code $LASTEXITCODE." }
}

Push-Location -LiteralPath $repositoryRoot
try {
    $env:RUSTFLAGS = ($previousRustFlags + ' -C target-feature=+crt-static').Trim()
    if (-not $SkipRustChecks) {
        & (Join-Path $PSScriptRoot 'update-package-catalog.ps1') -Check
        Invoke-TestCargo -CargoArguments @('fmt', '--check')
        Invoke-TestCargo -CargoArguments @('clippy', '--locked', '--all-targets', '--', '-D', 'warnings')
        Invoke-TestCargo -CargoArguments @('test', '--locked')
    }
    if ([string]::IsNullOrWhiteSpace($Executable)) {
        Invoke-TestCargo -CargoArguments @('build', '--release', '--locked')
        $metadataText = & cargo metadata --locked --format-version 1 --no-deps
        if ($LASTEXITCODE -ne 0) { throw 'Impossible de lire les métadonnées Cargo.' }
        $metadata = $metadataText | ConvertFrom-Json
        $Executable = Join-Path $metadata.target_directory 'release\supermenu.exe'
    }
    $Executable = [IO.Path]::GetFullPath($Executable)
    if (-not [IO.File]::Exists($Executable)) { throw "Exécutable absent : $Executable" }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    foreach ($suite in @('launcher', 'about', 'translations', 'helper-console')) {
        & (Join-Path $repositoryRoot "tests\windows\$suite.ps1") -Executable $Executable -OutputDirectory $OutputDirectory -Capture:$Capture
        if (-not $?) { throw "Échec de la suite native $suite." }
    }
    Write-Host "Tests Windows réussis. Rapports : $OutputDirectory"
}
finally {
    $env:RUSTFLAGS = $previousRustFlags
    Pop-Location
}
