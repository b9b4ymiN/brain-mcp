# Phase E1 DoD #1: no mock/TODO path in production build.
#
# Builds the Console SPA (web/console) and greps the built bundle for the
# forbidden markers. Fails the gate (exit non-zero) if any match is found.
#
# Forbidden patterns (case-insensitive):
#   TODO | FIXME | MOCK_DATA | mock_
#
# Rationale (Phase E1 Gate §13 Task 5.1): every Console page must call the
# REAL Rust API; no mock data, no leftover TODO/FIXME markers may ship in the
# production bundle. This is the load-bearing DoD #1 assertion — a TypeScript
# grep of the source could be satisfied by a commented-out TODO, so we grep
# the BUILT bundle (the bytes the browser actually runs) to be airtight.
#
# Usage:
#   pwsh scripts/console_grep_gate.ps1
# Exit codes:
#   0 = clean (no forbidden markers in the built bundle)
#   1 = found one or more matches (printed with file + line)

[CmdletBinding()]
param(
    # Skip the npm build if you've already run `npm run build` and want to
    # re-grep the existing dist/.
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Definition
$consoleDir = Join-Path $scriptRoot '..\web\console' | Resolve-Path
$distAssets = Join-Path $consoleDir 'dist\assets'

if (-not $SkipBuild) {
    Write-Host "[console_grep_gate] building console in $consoleDir"
    Push-Location $consoleDir
    try {
        npm run build
        if ($LASTEXITCODE -ne 0) {
            Write-Error "[console_grep_gate] npm run build failed (exit $LASTEXITCODE)"
            exit 1
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path $distAssets)) {
    Write-Error "[console_grep_gate] $distAssets does not exist — run without -SkipBuild first."
    exit 1
}

$jsFiles = Get-ChildItem -Path $distAssets -Filter '*.js' -File
if ($jsFiles.Count -eq 0) {
    Write-Error "[console_grep_gate] no .js files found in $distAssets — build may be misconfigured."
    exit 1
}

$pattern = '(?i)(TODO|FIXME|MOCK_DATA|mock_)'
# IMPORTANT: this accumulator MUST NOT be named $matches/$Matches — PowerShell's
# `-match` operator populates the automatic $Matches variable (case-insensitive
# name), which would clobber this array on the first hit and turn the `+=` into
# a "A hash table can only be added to another hash table" stack trace. Use
# $found; the automatic $Matches[0] below is intentional (matched text capture).
$found = @()
foreach ($file in $jsFiles) {
    $lineNum = 0
    foreach ($line in Get-Content $file.FullName) {
        $lineNum++
        if ($line -match $pattern) {
            $found += [pscustomobject]@{
                File   = $file.FullName
                Line   = $lineNum
                Match  = $Matches[0]
                Source = $line.Trim()
            }
        }
    }
}

if ($found.Count -gt 0) {
    Write-Host ""
    Write-Host "[console_grep_gate] FAIL: $($found.Count) forbidden marker(s) in built bundle:" -ForegroundColor Red
    $found | Format-Table -AutoSize
    Write-Host "Phase E1 DoD #1 NOT satisfied — remove the markers and rebuild." -ForegroundColor Red
    exit 1
}

Write-Host "[console_grep_gate] PASS: no TODO/FIXME/MOCK_DATA/mock_ markers in $($jsFiles.Count) built .js file(s)." -ForegroundColor Green
exit 0
