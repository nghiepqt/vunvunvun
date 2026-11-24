#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Fails if any app-defined colour pair in ui/tokens/palette.json falls below its WCAG contrast minimum.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $RepositoryRoot) { $RepositoryRoot = Split-Path -Parent $scriptRoot }

$pairsFile   = Join-Path $scriptRoot 'contrast-pairs.json'
$paletteFile = Join-Path $RepositoryRoot 'ui/tokens/palette.json'

if (-not (Test-Path $pairsFile))   { Write-Error "Missing $pairsFile."; exit 1 }
if (-not (Test-Path $paletteFile)) { Write-Error "Missing $paletteFile."; exit 1 }

$config  = Get-Content -LiteralPath $pairsFile -Raw | ConvertFrom-Json
$palette = Get-Content -LiteralPath $paletteFile -Raw | ConvertFrom-Json

function ConvertTo-Rgb([string] $Value) {
    $v = $Value.Trim()
    if (-not $v.StartsWith('#')) { return $null }
    $hex = $v.Substring(1)
    if ($hex.Length -eq 8) { $hex = $hex.Substring(2) }
    if ($hex.Length -ne 6) { return $null }
    return @(
        [Convert]::ToInt32($hex.Substring(0, 2), 16)
        [Convert]::ToInt32($hex.Substring(2, 2), 16)
        [Convert]::ToInt32($hex.Substring(4, 2), 16)
    )
}

function Get-RelativeLuminance([int[]] $Rgb) {
    $channels = foreach ($c in $Rgb) {
        $s = $c / 255.0
        if ($s -le 0.03928) { $s / 12.92 } else { [Math]::Pow(($s + 0.055) / 1.055, 2.4) }
    }
    return 0.2126 * $channels[0] + 0.7152 * $channels[1] + 0.0722 * $channels[2]
}

function Get-ContrastRatio([int[]] $A, [int[]] $B) {
    $la = Get-RelativeLuminance $A
    $lb = Get-RelativeLuminance $B
    $lighter = [Math]::Max($la, $lb)
    $darker  = [Math]::Min($la, $lb)
    return ($lighter + 0.05) / ($darker + 0.05)
}

$pairs = @($config.pairs)
if ($pairs.Count -eq 0) {
    Write-Error 'No contrast pairs declared in build/contrast-pairs.json.'
    exit 1
}

$failures = [System.Collections.Generic.List[string]]::new()

foreach ($themeProp in $palette.themes.PSObject.Properties) {
    $themeName = $themeProp.Name
    $tokens = $themeProp.Value

    foreach ($pair in $pairs) {
        $kind = if ($pair.PSObject.Properties.Name -contains 'kind') { $pair.kind } else { 'text' }
        $minimum = $config.minimumRatios.$kind
        if (-not $minimum) {
            $failures.Add("[$themeName] Unknown contrast kind '$kind'.")
            continue
        }

        $fgHex = $tokens.($pair.foreground)
        $bgHex = $tokens.($pair.background)
        if (-not $fgHex -or -not $bgHex) {
            $failures.Add("[$themeName] Missing token '$($pair.foreground)' or '$($pair.background)'.")
            continue
        }

        $fg = ConvertTo-Rgb $fgHex
        $bg = ConvertTo-Rgb $bgHex
        $ratio = Get-ContrastRatio $fg $bg
        $rounded = [Math]::Round($ratio, 2)
        if ($ratio -lt $minimum) {
            $failures.Add("[$themeName] $($pair.foreground) on $($pair.background) is ${rounded}:1, below ${minimum}:1 ($kind).")
        } else {
            Write-Host ("  OK  [{0}] {1} on {2}: {3}:1 (needs {4}:1)" -f $themeName, $pair.foreground, $pair.background, $rounded, $minimum)
        }
    }
}

if ($failures.Count -gt 0) {
    Write-Host ''
    Write-Host 'Contrast check failed:' -ForegroundColor Red
    foreach ($f in $failures) { Write-Host "  $f" -ForegroundColor Red }
    exit 1
}

Write-Host 'All declared colour pairs across all themes meet WCAG contrast minimums.' -ForegroundColor Green
exit 0
