#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Stages and packs the Rust vun.exe binary and MSIX manifest/assets into an .msix package.
#>
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string] $Platform = 'x64',

    [Parameter(Mandatory = $true)]
    [string] $BinaryPath,

    [string] $OutputDir = 'artifacts/msix'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$resolvedBinary = (Resolve-Path -LiteralPath $BinaryPath).Path
if (-not (Test-Path $resolvedBinary)) {
    throw "Binary not found at $BinaryPath"
}

$manifestSrc = Join-Path $repoRoot 'packaging/msix/AppxManifest.xml'
$assetsSrc   = Join-Path $repoRoot 'packaging/msix/Assets'
$stageDir    = Join-Path $repoRoot "target/msix-stage/$Platform"
$outFullDir  = Join-Path $repoRoot $OutputDir

if (Test-Path $stageDir) { Remove-Item -LiteralPath $stageDir -Recurse -Force }
New-Item -ItemType Directory -Path $stageDir -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $stageDir 'Assets') -Force | Out-Null
New-Item -ItemType Directory -Path $outFullDir -Force | Out-Null

Copy-Item -LiteralPath $resolvedBinary -Destination (Join-Path $stageDir 'vun.exe') -Force
Copy-Item -Path (Join-Path $assetsSrc '*') -Destination (Join-Path $stageDir 'Assets') -Recurse -Force

[xml] $manifest = Get-Content -LiteralPath $manifestSrc -Raw
$manifest.Package.Identity.ProcessorArchitecture = $Platform
$stagedManifest = Join-Path $stageDir 'AppxManifest.xml'
$manifest.Save($stagedManifest)

# Locate makeappx.exe from Windows 10/11 SDK
$kitsRoot = "${env:ProgramFiles(x86)}\Windows Kits\10\bin"
$makeAppx = Get-ChildItem -Path $kitsRoot -Recurse -Filter 'makeappx.exe' -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\x64\\makeappx\.exe$' } |
    Sort-Object FullName -Descending |
    Select-Object -First 1

if (-not $makeAppx) {
    throw "makeappx.exe not found under $kitsRoot. Install the Windows 10/11 SDK."
}

$outMsix = Join-Path $outFullDir "Vun_$Platform.msix"
if (Test-Path $outMsix) { Remove-Item -LiteralPath $outMsix -Force }

& $makeAppx.FullName pack /d $stageDir /p $outMsix /o
if ($LASTEXITCODE -ne 0) {
    throw "makeappx.exe failed with exit code $LASTEXITCODE"
}

Write-Host "Packed MSIX: $outMsix"
