# Feature checklist CI verifier and milestone progress report (plan K.27, Appendix C.3.2 & C.3.3).
# Checks that every row in docs/features.toml has a valid status, and that every `shipped` row names
# at least one test that resolves in `crates/` (or `.github/workflows/` for CI-level checks).
param(
    [string] $TomlPath = (Join-Path (Split-Path -Parent $PSScriptRoot) 'docs/features.toml'),
    [switch] $StrictShipped
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$lines = Get-Content $TomlPath
$entries = @()
$current = $null

foreach ($raw in $lines) {
    $line = $raw.Trim()
    if ($line -eq '' -or $line.StartsWith('#')) { continue }
    if ($line -match '^\["([^"]+)"\]$') {
        if ($null -ne $current) { $entries += $current }
        $current = @{
            Tag = $Matches[1]
            Label = ''
            Status = ''
            Tests = @()
            Open = ''
        }
        continue
    }
    if ($null -eq $current) { continue }
    if ($line -match '^label\s*=\s*"([^"]*)"') {
        $current.Label = $Matches[1]
    } elseif ($line -match '^status\s*=\s*"([^"]*)"') {
        $current.Status = $Matches[1]
    } elseif ($line -match '^open\s*=\s*"(.*)"$') {
        $current.Open = $Matches[1]
    } elseif ($line -match '^tests\s*=\s*\[(.*)\]$') {
        $inner = $Matches[1]
        $matches = [regex]::Matches($inner, '"([^"]+)"')
        foreach ($m in $matches) {
            $current.Tests += $m.Groups[1].Value
        }
    }
}
if ($null -ne $current) { $entries += $current }

$validStatuses = @('planned', 'in-progress', 'shipped', 'dropped')
$allRustFiles = Get-ChildItem -Recurse -File (Join-Path $root 'crates') -Filter '*.rs'

$fnNames = New-Object 'System.Collections.Generic.HashSet[string]'
$modNames = New-Object 'System.Collections.Generic.HashSet[string]'
$integrationTestStems = New-Object 'System.Collections.Generic.HashSet[string]'
$filesWithModTests = New-Object 'System.Collections.Generic.List[string]'

foreach ($rf in $allRustFiles) {
    $content = Get-Content $rf.FullName -Raw
    $normPath = $rf.FullName -replace '\\', '/'
    foreach ($m in [regex]::Matches($content, '\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\b')) {
        [void]$fnNames.Add($m.Groups[1].Value)
    }
    foreach ($m in [regex]::Matches($content, '\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\b')) {
        [void]$modNames.Add($m.Groups[1].Value)
    }
    if ($normPath -match '/tests/([^/]+)\.rs$') {
        [void]$integrationTestStems.Add($Matches[1])
    }
    if ($content -match '\bmod\s+tests\b' -or $normPath -match '/tests/[^/]+\.rs$') {
        $filesWithModTests.Add($normPath)
    }
}

$errors = @()
$areaStats = @{}

foreach ($e in $entries) {
    $area = ($e.Tag -split '\.')[0]
    if (-not $areaStats.ContainsKey($area)) {
        $areaStats[$area] = @{ shipped = 0; 'in-progress' = 0; planned = 0; dropped = 0 }
    }
    if ($e.Status -notin $validStatuses) {
        $errors += "$($e.Tag): invalid status '$($e.Status)'"
        continue
    }
    $areaStats[$area][$e.Status]++

    if ($e.Status -eq 'shipped') {
        if ($e.Tests.Count -eq 0) {
            $errors += "$($e.Tag) (shipped): has empty tests = []"
            continue
        }
        foreach ($testRef in $e.Tests) {
            if ($testRef -match '\.yml\)') { continue }
            $clean = ($testRef -split '\s+')[0]
            $found = $false
            if ($clean -match '::') {
                $parts = $clean -split '::'
                $tail = $parts[-1]
                if ($tail -eq 'tests') {
                    $modPart = if ($parts.Count -ge 2) { $parts[-2] } else { '' }
                    $cratePart = $modPart -replace '_', '-'
                    foreach ($p in $filesWithModTests) {
                        if ($p -match "/$([regex]::Escape($modPart))(\.rs|/mod\.rs)$" -or
                            $p -match "/crates/$([regex]::Escape($cratePart))/") {
                            $found = $true
                            break
                        }
                    }
                    if (-not $found -and $modNames.Contains($modPart)) {
                        $found = $true
                    }
                } else {
                    $found = $fnNames.Contains($tail)
                }
            } else {
                $found = $fnNames.Contains($clean) -or $integrationTestStems.Contains($clean)
            }
            if (-not $found) {
                $errors += "$($e.Tag) (shipped): test '$testRef' not found in crates/"
            }
        }
    }
}

Write-Output "| Area | Shipped | In-Progress | Planned | Dropped |"
Write-Output "| --- | ---: | ---: | ---: | ---: |"
foreach ($area in ($areaStats.Keys | Sort-Object)) {
    $s = $areaStats[$area]
    Write-Output "| $area | $($s['shipped']) | $($s['in-progress']) | $($s['planned']) | $($s['dropped']) |"
}
Write-Output ""
Write-Output "Total tracked entries: $($entries.Count); validation warnings/errors: $($errors.Count)"

if ($errors.Count -gt 0) {
    $errors | ForEach-Object { Write-Warning $_ }
    if ($StrictShipped) {
        exit 1
    }
}
