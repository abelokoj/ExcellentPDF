# Feature checklist CI verifier and milestone progress report (plan K.27, Appendix C.3.2 & C.3.3).
# Checks that:
# 1. Every row in docs/features.toml has a valid status and a matching doc in docs/features/<TAG>-*.md.
# 2. Every `shipped` and `in-progress` row resolves every test/workflow/script it names (`shipped` also requires non-empty tests).
# 3. No `Mock[A-Z]` identifier appears in non-test production code under `crates/ecclesia-app/src/`.
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
$featureDocsDir = Join-Path $root 'docs/features'
$featureDocNames = @()
if (Test-Path $featureDocsDir) {
    $featureDocNames = Get-ChildItem -File $featureDocsDir -Filter '*.md' | Select-Object -ExpandProperty Name
}

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

function Test-FeatureRefResolved {
    param([string] $TestRef)

    # File-path reference inside parentheses, e.g. "budgets (.github/workflows/windows.yml)"
    if ($TestRef -match '\(([^)]+\.(?:yml|yaml|ps1|sh))\)') {
        $rel = $Matches[1]
        return (Test-Path (Join-Path $root $rel))
    }

    # Direct relative path reference, e.g. ".github/workflows/macos.yml" or "packaging/windows/test-msi.ps1"
    if ($TestRef -match '^[A-Za-z0-9._/\-]+\.(?:yml|yaml|ps1|sh)$') {
        return (Test-Path (Join-Path $root $TestRef))
    }

    $clean = ($TestRef -split '\s+')[0]
    if ($clean -match '::') {
        $parts = $clean -split '::'
        $tail = $parts[-1]
        if ($tail -eq 'tests') {
            $modPart = if ($parts.Count -ge 2) { $parts[-2] } else { '' }
            $parentPart = if ($parts.Count -ge 3) { $parts[-3] } else { '' }
            $cratePart = $modPart -replace '_', '-'
            foreach ($p in $filesWithModTests) {
                if ($parentPart -ne '' -and $parentPart -ne 'crate' -and $parentPart -notmatch '^ecclesia_') {
                    if ($p -match "/$([regex]::Escape($parentPart))/$([regex]::Escape($modPart))(\.rs|/mod\.rs)$") {
                        return $true
                    }
                } else {
                    if ($p -match "/$([regex]::Escape($modPart))(\.rs|/mod\.rs)$" -or
                        $p -match "/crates/$([regex]::Escape($cratePart))/") {
                        return $true
                    }
                }
            }
            return $false
        }
        return $fnNames.Contains($tail)
    }
    return ($fnNames.Contains($clean) -or $integrationTestStems.Contains($clean))
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

    # Verify matching feature doc exists in docs/features/<TAG>-*.md.
    $docPrefix = "$($e.Tag)-"
    $hasDoc = $false
    foreach ($docName in $featureDocNames) {
        if ($docName.StartsWith($docPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            $hasDoc = $true
            break
        }
    }
    if (-not $hasDoc) {
        $errors += "$($e.Tag): missing feature doc in docs/features/$($e.Tag)-*.md"
    }

    if ($e.Status -eq 'shipped' -or $e.Status -eq 'in-progress') {
        if ($e.Status -eq 'shipped' -and $e.Tests.Count -eq 0) {
            $errors += "$($e.Tag) (shipped): has empty tests = []"
            continue
        }
        foreach ($testRef in $e.Tests) {
            if (-not (Test-FeatureRefResolved -TestRef $testRef)) {
                $errors += "$($e.Tag) ($($e.Status)): test '$testRef' not found"
            }
        }
    }
}

# Guardrail (F5): no Mock[A-Z] identifier may appear in non-test production code of crates/ecclesia-app/src/.
$appSrcDir = Join-Path $root 'crates/ecclesia-app/src'
if (Test-Path $appSrcDir) {
    $appRustFiles = Get-ChildItem -Recurse -File $appSrcDir -Filter '*.rs'
    foreach ($af in $appRustFiles) {
        $fileLines = Get-Content $af.FullName
        $pendingCfgTest = $false
        $inTestBlock = $false
        $braceDepth = 0
        for ($idx = 0; $idx -lt $fileLines.Count; $idx++) {
            $rawLine = $fileLines[$idx]
            # Strip single-line comments before scanning identifiers and braces.
            $code = $rawLine -replace '//.*$', ''
            if (-not $inTestBlock) {
                if ($code -match '#\[\s*cfg\s*\(\s*test\s*\)\s*\]') {
                    $pendingCfgTest = $true
                    continue
                }
                if ($pendingCfgTest) {
                    if ($code.Trim() -eq '' -or $code.Trim().StartsWith('#[')) {
                        continue
                    }
                    $opens = ([regex]::Matches($code, '\{')).Count
                    $closes = ([regex]::Matches($code, '\}')).Count
                    if ($opens -gt 0) {
                        $inTestBlock = $true
                        $braceDepth = $opens - $closes
                        if ($braceDepth -le 0) {
                            $inTestBlock = $false
                            $braceDepth = 0
                        }
                    }
                    $pendingCfgTest = $false
                    continue
                }
                $m = [regex]::Match($code, '\bMock[A-Z][A-Za-z0-9_]*\b')
                if ($m.Success) {
                    $relPath = $af.FullName.Substring($root.Length + 1) -replace '\\', '/'
                    $errors += "production mock forbidden: $relPath`:$($idx + 1) references '$($m.Value)' outside #[cfg(test)]"
                }
            } else {
                $opens = ([regex]::Matches($code, '\{')).Count
                $closes = ([regex]::Matches($code, '\}')).Count
                $braceDepth += ($opens - $closes)
                if ($braceDepth -le 0) {
                    $inTestBlock = $false
                    $braceDepth = 0
                }
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
