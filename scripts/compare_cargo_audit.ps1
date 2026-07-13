param(
    [Parameter(Mandatory = $true)][string]$Before,
    [Parameter(Mandatory = $true)][string]$After
)

$ErrorActionPreference = "Stop"

function Get-AuditFindingSet([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "cargo-audit report not found: $Path"
    }
    $document = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    $set = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::Ordinal)

    foreach ($item in @($document.vulnerabilities.list)) {
        [void]$set.Add("vulnerable|$($item.advisory.id)|$($item.package.name)|$($item.package.version)")
    }

    if ($null -ne $document.warnings) {
        foreach ($category in $document.warnings.PSObject.Properties) {
            foreach ($item in @($category.Value)) {
                $kind = if ($item.kind) { [string]$item.kind } else { [string]$category.Name }
                $advisory = if ($item.advisory.id) { [string]$item.advisory.id } else { "no-advisory-id" }
                $packageName = if ($item.package.name) { [string]$item.package.name } else { "no-package" }
                $packageVersion = if ($item.package.version) { [string]$item.package.version } else { "no-version" }
                [void]$set.Add("$kind|$advisory|$packageName|$packageVersion")
            }
        }
    }
    return $set
}

$beforeSet = Get-AuditFindingSet $Before
$afterSet = Get-AuditFindingSet $After
$newFindings = @($afterSet | Where-Object { -not $beforeSet.Contains($_) } | Sort-Object)

Write-Output "cargo-audit findings before=$($beforeSet.Count) after=$($afterSet.Count)"
if ($newFindings.Count -gt 0) {
    throw ("New cargo-audit findings are forbidden:`n" + ($newFindings -join "`n"))
}
Write-Output "No new vulnerable, unsound, unmaintained, notice, or yanked findings."
