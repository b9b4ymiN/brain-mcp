param(
    [Parameter(Mandatory = $true)][string]$Report,
    [string]$PathPrefix = "src/semantic",
    [double]$MinimumLines = 80
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Report -PathType Leaf)) {
    throw "Coverage report not found: $Report"
}

$document = Get-Content -LiteralPath $Report -Raw | ConvertFrom-Json
$normalizedPrefix = $PathPrefix.Replace("\", "/").TrimEnd("/")
$files = @($document.data[0].files | Where-Object {
    $name = $_.filename.Replace("\", "/")
    $name.EndsWith(".rs") -and $name.Contains("/$normalizedPrefix")
})

if ($files.Count -eq 0) {
    throw "No Rust coverage entries matched path prefix '$PathPrefix'"
}

$failed = @()
foreach ($file in $files) {
    $count = [int]$file.summary.lines.count
    $covered = [int]$file.summary.lines.covered
    $percent = [double]$file.summary.lines.percent
    if ($count -le 0) {
        $failed += "$($file.filename): no executable lines"
        continue
    }
    Write-Output ("{0}: {1:N2}% lines ({2}/{3})" -f $file.filename, $percent, $covered, $count)
    if ($percent -lt $MinimumLines) {
        $failed += ("$($file.filename): {0:N2}% < {1:N2}%" -f $percent, $MinimumLines)
    }
}

if ($failed.Count -gt 0) {
    throw ("Semantic per-file coverage gate failed:`n" + ($failed -join "`n"))
}

Write-Output "Semantic per-file line coverage gate passed for $($files.Count) file(s)."
