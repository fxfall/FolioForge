param(
    [Parameter(Mandatory = $true)]
    [string]$Target,
    [Parameter(Mandatory = $true)]
    [string]$Label,
    [Parameter(Mandatory = $true)]
    [string]$OutputRoot
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$validationRoot = $env:FOLIOFORGE_VALIDATION_ROOT
if ([string]::IsNullOrWhiteSpace($validationRoot)) {
    throw "FOLIOFORGE_VALIDATION_ROOT must point outside the repository"
}
if ($Target -notin @("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc")) {
    throw "unsupported Windows Core target: $Target"
}
if ($Label -notin @("windows-x86_64", "windows-aarch64")) {
    throw "unsupported Core artifact label: $Label"
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmssfff"
$buildRoot = Join-Path $validationRoot "core-$Label-$stamp"
$cargoTarget = Join-Path $buildRoot "cargo-target"
$tempRoot = Join-Path $buildRoot "tmp"
$runtimeRoot = Join-Path $buildRoot "runtime"
$artifactRoot = (New-Item -ItemType Directory -Force -Path $OutputRoot).FullName
New-Item -ItemType Directory -Force -Path $cargoTarget, $tempRoot, $runtimeRoot | Out-Null

$env:CARGO_TARGET_DIR = $cargoTarget
$env:TEMP = $tempRoot
$env:TMP = $tempRoot
$env:FOLIOFORGE_TEMP_ROOT = $runtimeRoot

try {
    & rustup target add $Target
    if ($LASTEXITCODE -ne 0) { throw "rustup target add failed" }
    & cargo test --locked -p folio-cli -p folio-core
    if ($LASTEXITCODE -ne 0) { throw "Core test suite failed" }
    & cargo build --locked --release -p folio-cli --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Core release build failed" }

    $binary = Join-Path $cargoTarget "$Target\release\folio.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw "folio.exe was not produced at $binary"
    }

    $versionOutput = (& $binary --version | Out-String).Trim()
    Write-Host $versionOutput
    if ($versionOutput -notmatch "0\.1\.0") {
        throw "unexpected FolioForge version: $versionOutput"
    }
    & $binary --help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "folio --help failed" }

    $smokeRoot = Join-Path $buildRoot "smoke"
    New-Item -ItemType Directory -Force -Path $smokeRoot | Out-Null
    $smokeInput = Join-Path $smokeRoot "图书.epub"
    $smokeOutput = Join-Path $smokeRoot "输出.azw3"
    & python (Join-Path $root "tests\scripts\build_smoke_fixture.py") $smokeInput
    & $binary convert $smokeInput --to kf8 --mode compatible --output $smokeOutput *> (Join-Path $smokeRoot "convert.log")
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $smokeOutput -PathType Leaf)) {
        Get-Content (Join-Path $smokeRoot "convert.log")
        throw "Windows Core smoke conversion failed"
    }
    & $binary validate $smokeOutput *> (Join-Path $smokeRoot "validate.json")
    if ($LASTEXITCODE -ne 0) {
        throw "Windows Core smoke validation failed"
    }
    & $binary inspect $smokeOutput --semantic *> (Join-Path $smokeRoot "semantic.json")
    if ($LASTEXITCODE -ne 0) {
        throw "Windows Core semantic smoke inspection failed"
    }
    if (Get-ChildItem -LiteralPath $buildRoot -Recurse -File | Where-Object { $_.Name -match '^library\.sqlite(3)?$' -or $_.Extension -in @('.sqlite', '.sqlite3') }) {
        throw "Core smoke unexpectedly created a Library database"
    }

    $packageName = "FolioForge-Core-0.1.0-$Label"
    $packageRoot = Join-Path $buildRoot "package\$packageName"
    New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot "bin") | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $packageRoot "bin\folio.exe")
    Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $packageRoot "LICENSE")

    $archive = Join-Path $artifactRoot "$packageName.zip"
    Compress-Archive -Path $packageRoot -DestinationPath $archive -CompressionLevel Optimal -Force
    $verifyRoot = Join-Path $buildRoot "verify"
    Expand-Archive -LiteralPath $archive -DestinationPath $verifyRoot -Force
    if (-not (Test-Path -LiteralPath (Join-Path $verifyRoot "$packageName\bin\folio.exe") -PathType Leaf)) {
        throw "Windows Core archive does not contain the CLI"
    }
    if (Get-ChildItem -LiteralPath $verifyRoot -Recurse -File | Where-Object { $_.Name -match '^library\.sqlite(3)?$' -or $_.Extension -in @('.sqlite', '.sqlite3') }) {
        throw "Windows Core archive contains a Library database"
    }

    $semanticPath = Join-Path $smokeRoot "semantic.json"
    $semanticHash = Join-Path $artifactRoot "$packageName.zip.smoke.semantic.sha256"
    & python (Join-Path $root "tests\scripts\canonical_json_hash.py") $semanticPath $semanticHash
    if ($LASTEXITCODE -ne 0) {
        throw "semantic smoke hash failed"
    }

    Write-Host "Built and smoke-tested $archive"
}
finally {
    if (Test-Path -LiteralPath $buildRoot) {
        Remove-Item -LiteralPath $buildRoot -Recurse -Force
    }
}
