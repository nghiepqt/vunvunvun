#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Fails if user-facing copy contains vocabulary that would frame this product as
    medical or therapeutic (CONTEXT.md section 6).

.DESCRIPTION
    Scope is deliberately narrow: the localization catalog and store metadata only.
    Running this over identifiers or comments would fail on the project's own domain
    vocabulary, which is explicitly permitted.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Computed here rather than as a param default: $PSScriptRoot is not reliably bound
# inside a param block across PowerShell editions.
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $RepositoryRoot) { $RepositoryRoot = Split-Path -Parent $scriptRoot }

$bannedFile    = Join-Path $scriptRoot 'banned-vocabulary.txt'
$allowlistFile = Join-Path $scriptRoot 'banned-vocabulary-allowlist.txt'

function Read-ListFile([string] $Path) {
    if (-not (Test-Path $Path)) { return @() }
    Get-Content -LiteralPath $Path |
        ForEach-Object { ($_ -split '#', 2)[0].Trim() } |
        Where-Object { $_ -ne '' }
}

$banned    = @(Read-ListFile $bannedFile)
$allowlist = @(Read-ListFile $allowlistFile)

if ($banned.Count -eq 0) {
    Write-Error "No banned words loaded from $bannedFile."
    exit 1
}

# User-facing surfaces only.
$targets = @()
$targets += Get-ChildItem -Path $RepositoryRoot -Recurse -Filter '*.resw' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '[\/](bin|obj)[\/]' }
$targets += Get-ChildItem -Path $RepositoryRoot -Recurse -Filter 'Package.appxmanifest' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '[\/](bin|obj)[\/]' }

if ($targets.Count -eq 0) {
    Write-Error 'No localization catalog or store metadata found. The check has nothing to scan, which is itself a defect.'
    exit 1
}

$findings = [System.Collections.Generic.List[object]]::new()
$scanned  = 0

foreach ($file in $targets) {
    [xml] $xml = Get-Content -LiteralPath $file.FullName -Raw

    # Resource values from a .resw, plus the store-visible strings from a manifest.
    $strings = @()
    $strings += $xml.SelectNodes('//data/value')          | ForEach-Object { $_.InnerText }
    $strings += $xml.SelectNodes('//*[local-name()="DisplayName"]')            | ForEach-Object { $_.InnerText }
    $strings += $xml.SelectNodes('//*[local-name()="PublisherDisplayName"]')   | ForEach-Object { $_.InnerText }
    $strings += $xml.SelectNodes('//*[local-name()="VisualElements"]/@DisplayName')  | ForEach-Object { $_.Value }
    $strings += $xml.SelectNodes('//*[local-name()="VisualElements"]/@Description')  | ForEach-Object { $_.Value }
    $strings = $strings | Where-Object { $_ -and $_.Trim() -ne '' }

    foreach ($text in $strings) {
        $scanned++
        if ($allowlist | Where-Object { $_ -ieq $text.Trim() }) { continue }

        foreach ($word in $banned) {
            if ($text -imatch "\b$([regex]::Escape($word))\b") {
                $findings.Add([pscustomobject]@{
                    File = $file.FullName.Substring($RepositoryRoot.Length).TrimStart([char]'\', [char]'/')
                    Word = $word
                    Text = $text.Trim()
                })
            }
        }
    }
}

Write-Host "Scanned $scanned user-facing strings across $($targets.Count) file(s) for $($banned.Count) banned words."

if ($findings.Count -gt 0) {
    Write-Host ''
    Write-Host 'Banned vocabulary found in user-facing copy:' -ForegroundColor Red
    foreach ($f in $findings) {
        Write-Host ("  {0}: '{1}' in `"{2}`"" -f $f.File, $f.Word, $f.Text) -ForegroundColor Red
    }
    Write-Host ''
    Write-Host 'Say "a short break" not "an intervention"; "helps you get started" not "treats executive dysfunction".'
    Write-Host "If a usage is genuinely not a medical claim, add the exact string to $allowlistFile with a note."
    exit 1
}

Write-Host 'No banned vocabulary in user-facing copy.' -ForegroundColor Green
exit 0
