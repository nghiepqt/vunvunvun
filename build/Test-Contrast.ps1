#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Fails if any app-defined colour pair falls below its WCAG contrast minimum.

.DESCRIPTION
    CONTEXT.md section 4.1 requires text at 4.5:1 and UI components at 3:1, automated
    in CI. Colours are read from <Color> and <SolidColorBrush> resources declared in
    the app's XAML; the pairings to check are declared in contrast-pairs.json, because
    which foreground renders on which background cannot be inferred from markup.
#>
[CmdletBinding()]
param(
    [string] $RepositoryRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $RepositoryRoot) { $RepositoryRoot = Split-Path -Parent $scriptRoot }

$pairsFile = Join-Path $scriptRoot 'contrast-pairs.json'
if (-not (Test-Path $pairsFile)) {
    Write-Error "Missing $pairsFile."
    exit 1
}

$config = Get-Content -LiteralPath $pairsFile -Raw | ConvertFrom-Json

function ConvertTo-Rgb([string] $Value) {
    $v = $Value.Trim()
    if (-not $v.StartsWith('#')) { return $null }
    $hex = $v.Substring(1)
    if ($hex.Length -eq 8) { $hex = $hex.Substring(2) }   # strip alpha from #AARRGGBB
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
    $darker   = [Math]::Min($la, $lb)
    return ($lighter + 0.05) / ($darker + 0.05)
}

# Collect every app-defined colour resource.
$colours = @{}
$xamlFiles = Get-ChildItem -Path (Join-Path $RepositoryRoot 'src') -Recurse -Filter '*.xaml' -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '[\/](bin|obj)[\/]' }

foreach ($file in $xamlFiles) {
    $text = Get-Content -LiteralPath $file.FullName -Raw
    foreach ($m in [regex]::Matches($text, '<Color\s+x:Key="(?<key>[^"]+)"\s*>(?<val>[^<]+)</Color>')) {
        $colours[$m.Groups['key'].Value] = $m.Groups['val'].Value
    }
    foreach ($m in [regex]::Matches($text, '<SolidColorBrush\s+x:Key="(?<key>[^"]+)"\s+Color="(?<val>#[0-9A-Fa-f]+)"')) {
        $colours[$m.Groups['key'].Value] = $m.Groups['val'].Value
    }
}

$pairs = @($config.pairs)
Write-Host "Found $($colours.Count) app-defined colour resource(s); $($pairs.Count) pair(s) declared for checking."

if ($pairs.Count -eq 0) {
    if ($colours.Count -gt 0) {
        Write-Host ''
        Write-Host 'App-defined colours exist but no pairs are declared, so nothing was verified.' -ForegroundColor Red
        Write-Host "Declare each foreground/background combination in $pairsFile."
        exit 1
    }
    Write-Host 'No app-defined colours yet; the app uses stock WinUI theme brushes. Nothing to check.' -ForegroundColor Yellow
    exit 0
}

$failures = [System.Collections.Generic.List[string]]::new()

foreach ($pair in $pairs) {
    $kind = if ($pair.PSObject.Properties.Name -contains 'kind') { $pair.kind } else { 'text' }
    $minimum = $config.minimumRatios.$kind
    if (-not $minimum) {
        $failures.Add("Pair '$($pair.foreground)' on '$($pair.background)' declares unknown kind '$kind'.")
        continue
    }

    foreach ($key in @($pair.foreground, $pair.background)) {
        if (-not $colours.ContainsKey($key)) {
            $failures.Add("Colour resource '$key' is referenced in contrast-pairs.json but not defined in any XAML.")
        }
    }
    if ($failures.Count -gt 0 -and -not ($colours.ContainsKey($pair.foreground) -and $colours.ContainsKey($pair.background))) { continue }

    $fg = ConvertTo-Rgb $colours[$pair.foreground]
    $bg = ConvertTo-Rgb $colours[$pair.background]
    if (-not $fg -or -not $bg) {
        $failures.Add("Could not parse colours for '$($pair.foreground)' on '$($pair.background)'.")
        continue
    }

    $ratio = Get-ContrastRatio $fg $bg
    $rounded = [Math]::Round($ratio, 2)
    if ($ratio -lt $minimum) {
        $failures.Add("$($pair.foreground) on $($pair.background) is ${rounded}:1, below the ${minimum}:1 minimum for $kind.")
    } else {
        Write-Host ("  OK  {0} on {1}: {2}:1 (needs {3}:1)" -f $pair.foreground, $pair.background, $rounded, $minimum)
    }
}

if ($failures.Count -gt 0) {
    Write-Host ''
    Write-Host 'Contrast check failed:' -ForegroundColor Red
    foreach ($f in $failures) { Write-Host "  $f" -ForegroundColor Red }
    exit 1
}

Write-Host 'All declared colour pairs meet their contrast minimums.' -ForegroundColor Green
exit 0
