#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Fails if user-facing copy contains vocabulary that would frame this product as
    medical or therapeutic (CONTEXT.md section 6).

.DESCRIPTION
    Scope is deliberately narrow: the i18n JSON catalog and MSIX store metadata only.
    Running this over identifiers or comments would fail on the project's own domain
    vocabulary, which is explicitly permitted.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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

function Get-JsonLeafStrings($Node) {
    $out = [System.Collections.Generic.List[string]]::new()
    if ($null -eq $Node) { return $out }
    if ($Node -is [string]) {
        $out.Add($Node)
        return $out
    }
    if ($Node -is [System.Collections.IEnumerable] -and -not ($Node -is [System.Collections.IDictionary])) {
        foreach ($item in $Node) {
            foreach ($s in (Get-JsonLeafStrings $item)) { $out.Add($s) }
        }
        return $out
    }
    foreach ($prop in $Node.PSObject.Properties) {
        foreach ($s in (Get-JsonLeafStrings $prop.Value)) { $out.Add($s) }
    }
    return $out
}

$banned    = @(Read-ListFile $bannedFile)
$allowlist = @(Read-ListFile $allowlistFile)

if ($banned.Count -eq 0) {
    Write-Error "No banned words loaded from $bannedFile."
    exit 1
}

$i18nFiles = @(Get-ChildItem -Path (Join-Path $RepositoryRoot 'i18n') -Filter '*.json' -File -ErrorAction SilentlyContinue)
$manifestFiles = @(Get-ChildItem -Path (Join-Path $RepositoryRoot 'packaging') -Recurse -Filter 'AppxManifest.xml' -File -ErrorAction SilentlyContinue)
$targets = @($i18nFiles) + @($manifestFiles)

if ($targets.Count -eq 0) {
    Write-Error 'No i18n JSON catalog or AppxManifest.xml found.'
    exit 1
}

$findings = [System.Collections.Generic.List[object]]::new()
$scanned  = 0

foreach ($file in $i18nFiles) {
    $json = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
    foreach ($text in (Get-JsonLeafStrings $json)) {
        if (-not $text -or $text.Trim() -eq '') { continue }
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

foreach ($file in $manifestFiles) {
    [xml] $xml = Get-Content -LiteralPath $file.FullName -Raw
    $strings = @()
    $strings += $xml.SelectNodes('//*[local-name()="DisplayName"]')          | ForEach-Object { $_.InnerText }
    $strings += $xml.SelectNodes('//*[local-name()="PublisherDisplayName"]') | ForEach-Object { $_.InnerText }
    $strings += $xml.SelectNodes('//*[local-name()="VisualElements"]/@DisplayName') | ForEach-Object { $_.Value }
    $strings += $xml.SelectNodes('//*[local-name()="VisualElements"]/@Description') | ForEach-Object { $_.Value }
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
    exit 1
}

Write-Host 'No banned vocabulary in user-facing copy.' -ForegroundColor Green
exit 0
