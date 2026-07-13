param(
    [string]$Binary = (Join-Path $PSScriptRoot '..\target\debug\llm-wiki.exe'),
    [string]$Fixture = (Join-Path $PSScriptRoot '..\tests\fixtures\wikis\research'),
    [string]$WorkRoot = (Join-Path ([System.IO.Path]::GetTempPath()) ("brain-vnext-restore-" + [guid]::NewGuid().ToString('N')))
)

$ErrorActionPreference = 'Stop'
$Binary = [System.IO.Path]::GetFullPath($Binary)
$Fixture = [System.IO.Path]::GetFullPath($Fixture)
$WorkRoot = [System.IO.Path]::GetFullPath($WorkRoot)

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    $previousErrorActionPreference = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 wraps native stderr as ErrorRecord. Git writes
        # successful clone progress to stderr, so defer failure to the exit code.
        $ErrorActionPreference = 'Continue'
        $output = & $Command @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw "Command failed ($exitCode): $Command $($Arguments -join ' ')`n$($output -join "`n")"
    }
    return @($output)
}

function Get-FileManifest {
    param([string]$Root)
    $rootPrefix = [System.IO.Path]::GetFullPath($Root)
    if (-not $rootPrefix.EndsWith([System.IO.Path]::DirectorySeparatorChar)) {
        $rootPrefix += [System.IO.Path]::DirectorySeparatorChar
    }
    return @(Get-ChildItem -LiteralPath $Root -Recurse -File |
        Where-Object { $_.FullName -notmatch '[\\/]\.git[\\/]' } |
        Sort-Object FullName |
        ForEach-Object {
            [pscustomobject]@{
                path = $_.FullName.Substring($rootPrefix.Length).Replace('\', '/')
                bytes = $_.Length
                sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash
            }
        })
}

function Get-CompositeHash {
    param([object[]]$Manifest)
    $canonical = ($Manifest | ForEach-Object { "$($_.path)`t$($_.bytes)`t$($_.sha256)" }) -join "`n"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($canonical)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $hash = $sha256.ComputeHash($bytes)
    }
    finally {
        $sha256.Dispose()
    }
    return ([System.BitConverter]::ToString($hash)).Replace('-', '')
}

function ConvertTo-CanonicalObject {
    param([object]$Value)
    if ($null -eq $Value -or $Value -is [string] -or $Value -is [ValueType]) {
        return $Value
    }
    if ($Value -is [System.Collections.IDictionary]) {
        $ordered = [ordered]@{}
        foreach ($key in @($Value.Keys | Sort-Object)) {
            $ordered[$key] = ConvertTo-CanonicalObject $Value[$key]
        }
        return $ordered
    }
    if ($Value -is [pscustomobject]) {
        $ordered = [ordered]@{}
        foreach ($property in @($Value.PSObject.Properties | Sort-Object Name)) {
            $ordered[$property.Name] = ConvertTo-CanonicalObject $property.Value
        }
        return $ordered
    }
    if ($Value -is [System.Collections.IEnumerable]) {
        return @($Value | ForEach-Object { ConvertTo-CanonicalObject $_ })
    }
    return $Value
}

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "Binary not found: $Binary"
}
if (-not (Test-Path -LiteralPath $Fixture -PathType Container)) {
    throw "Fixture not found: $Fixture"
}

$source = Join-Path $WorkRoot 'source'
$backup = Join-Path $WorkRoot 'backup.git'
$restored = Join-Path $WorkRoot 'restored'
$sourceConfig = Join-Path $WorkRoot 'source-config.toml'
$restoreConfig = Join-Path $WorkRoot 'restore-config.toml'
New-Item -ItemType Directory -Path $WorkRoot | Out-Null
Copy-Item -LiteralPath $Fixture -Destination $source -Recurse

Invoke-Checked git @('-C', $source, 'init', '--initial-branch=main') | Out-Null
Invoke-Checked git @('-C', $source, 'config', 'user.name', 'Brain vNext Baseline') | Out-Null
Invoke-Checked git @('-C', $source, 'config', 'user.email', 'baseline@localhost') | Out-Null
Invoke-Checked git @('-C', $source, 'add', '--all') | Out-Null
Invoke-Checked git @('-C', $source, 'commit', '-m', 'baseline restore fixture') | Out-Null
$sourceHead = (Invoke-Checked git @('-C', $source, 'rev-parse', 'HEAD') | Select-Object -Last 1).Trim()

Invoke-Checked $Binary @('--config', $sourceConfig, 'spaces', 'register', $source, '--name', 'baseline') | Out-Null
$sourceRebuild = Invoke-Checked $Binary @('--config', $sourceConfig, '--wiki', 'baseline', 'index', 'rebuild', '--format', 'json')
$sourceSearch = Invoke-Checked $Binary @('--config', $sourceConfig, '--wiki', 'baseline', 'search', 'mixture experts', '--format', 'json')
$sourceGraph = Invoke-Checked $Binary @('--config', $sourceConfig, '--wiki', 'baseline', 'graph', '--format', 'llms')

Invoke-Checked git @('clone', '--bare', '--no-local', $source, $backup) | Out-Null
Invoke-Checked git @('clone', $backup, $restored) | Out-Null
$restoreHead = (Invoke-Checked git @('-C', $restored, 'rev-parse', 'HEAD') | Select-Object -Last 1).Trim()

Invoke-Checked $Binary @('--config', $restoreConfig, 'spaces', 'register', $restored, '--name', 'baseline') | Out-Null
$restoreRebuild = Invoke-Checked $Binary @('--config', $restoreConfig, '--wiki', 'baseline', 'index', 'rebuild', '--format', 'json')
$restoreSearch = Invoke-Checked $Binary @('--config', $restoreConfig, '--wiki', 'baseline', 'search', 'mixture experts', '--format', 'json')
$restoreGraph = Invoke-Checked $Binary @('--config', $restoreConfig, '--wiki', 'baseline', 'graph', '--format', 'llms')

$sourceManifest = Get-FileManifest $source
$restoreManifest = Get-FileManifest $restored
$sourceHash = Get-CompositeHash $sourceManifest
$restoreHash = Get-CompositeHash $restoreManifest
$sourceSearchCanonical = ConvertTo-CanonicalObject (($sourceSearch -join "`n") | ConvertFrom-Json)
$restoreSearchCanonical = ConvertTo-CanonicalObject (($restoreSearch -join "`n") | ConvertFrom-Json)
$searchParity = (($sourceSearchCanonical | ConvertTo-Json -Depth 20 -Compress) -eq
    ($restoreSearchCanonical | ConvertTo-Json -Depth 20 -Compress))
$graphParity = (($sourceGraph -join "`n") -eq ($restoreGraph -join "`n"))
$passed = $sourceHead -eq $restoreHead -and
    $sourceManifest.Count -eq $restoreManifest.Count -and
    $sourceHash -eq $restoreHash -and
    $searchParity -and
    $graphParity

$result = [ordered]@{
    schema_version = 1
    passed = $passed
    work_root = $WorkRoot
    source_head = $sourceHead
    restore_head = $restoreHead
    file_count = $sourceManifest.Count
    source_composite_sha256 = $sourceHash
    restore_composite_sha256 = $restoreHash
    search_parity = $searchParity
    graph_parity = $graphParity
    source_rebuild = (($sourceRebuild -join "`n") | ConvertFrom-Json)
    restore_rebuild = (($restoreRebuild -join "`n") | ConvertFrom-Json)
    ledger_event_sequence = 'N/A: upstream baseline has no event ledger'
}

$manifestPath = Join-Path $WorkRoot 'restore-result.json'
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding utf8
$result | ConvertTo-Json -Depth 8
if (-not $passed) {
    exit 1
}
