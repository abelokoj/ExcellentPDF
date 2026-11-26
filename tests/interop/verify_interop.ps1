# Interop & Oracle Verifier (plan K.26, §16)
# Validates corpus PDFs (and any extra -PdfFiles passed) against structural PDF invariants and,
# when installed in PATH, external oracles `qpdf --check` and `pdftotext`.
param(
    [string[]] $PdfFiles = @()
)
$ErrorActionPreference = 'Stop'
$checklistPath = Join-Path $PSScriptRoot 'checklist.json'
$checklist = Get-Content $checklistPath -Raw | ConvertFrom-Json

$hasQpdf = $null -ne (Get-Command qpdf -ErrorAction SilentlyContinue)
$hasPdftotext = $null -ne (Get-Command pdftotext -ErrorAction SilentlyContinue)
$hasPdfsig = $null -ne (Get-Command pdfsig -ErrorAction SilentlyContinue)

$failures = @()
$checked = 0

foreach ($case in $checklist.cases) {
    $resolved = Resolve-Path (Join-Path $PSScriptRoot $case.file) -ErrorAction SilentlyContinue
    if (-not $resolved) {
        $failures += "$($case.id): missing file $($case.file)"
        continue
    }
    $bytes = [System.IO.File]::ReadAllBytes($resolved.Path)
    $text = [System.Text.Encoding]::GetEncoding(28591).GetString($bytes)
    foreach ($marker in $case.requires) {
        if (-not $text.Contains($marker)) {
            $failures += "$($case.id): missing marker '$marker'"
        }
    }
    if ($hasQpdf -and $case.check_qpdf) {
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & qpdf --check $resolved.Path 2>&1 | Out-Null
        $qpdfExit = $LASTEXITCODE
        $ErrorActionPreference = $prevEap
        if ($qpdfExit -ne 0) {
            $failures += "$($case.id): qpdf --check failed ($qpdfExit)"
        }
    }
    if ($hasPdftotext -and $case.check_pdftotext) {
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        $extracted = & pdftotext $resolved.Path - 2>&1
        $pdfTextExit = $LASTEXITCODE
        $ErrorActionPreference = $prevEap
        if ($pdfTextExit -ne 0 -or [string]::IsNullOrWhiteSpace(($extracted -join ' '))) {
            $failures += "$($case.id): pdftotext returned empty or failed ($pdfTextExit)"
        }
    }
    $checked++
}

foreach ($extra in $PdfFiles) {
    if (-not (Test-Path $extra)) {
        $failures += "extra: missing file $extra"
        continue
    }
    $text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes((Resolve-Path $extra).Path))
    if (-not ($text.Contains('%PDF-') -and $text.Contains('%%EOF'))) {
        $failures += "extra ($extra): missing %PDF- or %%EOF"
    }
    if ($hasQpdf) {
        & qpdf --check $extra *> $null
        if ($LASTEXITCODE -ne 0) {
            $failures += "extra ($extra): qpdf --check failed ($LASTEXITCODE)"
        }
    }
    $checked++
}

Write-Output "Interop checked $checked PDF(s) (oracles: qpdf=$hasQpdf, pdftotext=$hasPdftotext, pdfsig=$hasPdfsig)."
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Write-Error $_ }
    exit 1
}
