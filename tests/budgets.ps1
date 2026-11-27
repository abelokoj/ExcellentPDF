# Budget checks (plan §13.1, K.24), task T10+: measures a release build against tests/size/budgets.json and
# tests/perf/budgets.json, prints a table (and writes it to the GitHub job summary), and fails when a measurement is
# worse than its recorded baseline by more than the tolerance. Being over a budget is reported, not failed: Phase 0
# starts over some budgets, and only regressions block a pull request (AGENTS §7).
#
#   pwsh tests/budgets.ps1 -Exe target/release/ecclesiapdf.exe
#
# Measured: core installer size, cold start, opening a typical PDF, and idle memory with one small document.
param(
    [Parameter(Mandatory)] [string] $Exe,
    [int] $Runs = 7,
    # Default corpus document for the open & idle-memory budgets (§13.1).
    [string] $Document = (Join-Path (Split-Path -Parent $PSScriptRoot) 'tests/corpus/zlib-manual.pdf')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$sizeFile = Join-Path $root 'tests/size/budgets.json'
$perfFile = Join-Path $root 'tests/perf/budgets.json'
$size = Get-Content $sizeFile -Raw | ConvertFrom-Json
$perf = Get-Content $perfFile -Raw | ConvertFrom-Json

# Validate budget schemas before launching processes.
foreach ($requiredSize in @('core_installer_mb')) {
    if ($null -eq $size.budgets.$requiredSize.budget) {
        throw "tests/size/budgets.json missing budget '$requiredSize'"
    }
}
foreach ($requiredPerf in @(
    'cold_start_to_usable_window_ms',
    'open_typical_pdf_first_page_ms',
    'open_10000_page_pdf_first_page_ms',
    'scroll_and_page_turn_fps',
    'ink_pen_down_to_pixels_frames',
    'search_1000_pages_indexed_ms',
    'idle_memory_one_small_document_mb',
    'twenty_open_tabs_memory_mb',
    'ui_thread_stall_max_ms'
)) {
    if ($null -eq $perf.budgets.$requiredPerf.budget) {
        throw "tests/perf/budgets.json missing budget '$requiredPerf'"
    }
}

# Installer size. shortcut: until K.25 builds the MSI, the exe in a cabinet with LZX:21 (how an MSI stores its files,
# made with Windows' own makecab) stands in for it; the MSI adds its tables, tens of kilobytes.
$cabDir = Join-Path ([IO.Path]::GetTempPath()) "ecclesiapdf-budget-$PID"
New-Item -ItemType Directory -Force $cabDir | Out-Null
makecab /D CompressionType=LZX /D CompressionMemory=21 /L $cabDir $Exe ecclesiapdf.cab | Out-Null
if ($LASTEXITCODE -ne 0) { throw "makecab failed ($LASTEXITCODE)" }
$installerMb = [math]::Round((Get-Item (Join-Path $cabDir 'ecclesiapdf.cab')).Length / 1MB, 2)
Remove-Item -Recurse $cabDir

# Cold start: from process creation until the app has a visible top-level window, which NSH shows after its first
# frame. The median of $Runs launches, with no document.
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class BudgetWindows {
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  public static bool HasVisible(uint pid) {
    bool found = false;
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      if (p == pid && IsWindowVisible(h)) { found = true; return false; } return true; }, IntPtr.Zero);
    return found;
  }
}
"@
$times = @()
for ($i = 0; $i -lt $Runs; $i++) {
    $p = Start-Process -FilePath $Exe -PassThru
    $deadline = (Get-Date).AddSeconds(15)
    while (-not [BudgetWindows]::HasVisible([uint32]$p.Id)) {
        if ($p.HasExited -or (Get-Date) -gt $deadline) {
            if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
            throw "ecclesiapdf showed no window within 15 s (run $($i + 1))"
        }
    }
    $times += [int]((Get-Date) - $p.StartTime).TotalMilliseconds
    Stop-Process -Id $p.Id -Force
    $p.WaitForExit()
    Start-Sleep -Milliseconds 500
}
$coldStartMs = ($times | Sort-Object)[[int][math]::Floor($Runs / 2)]

# Open a typical PDF & idle working-set memory: the page canvas logs the time from being given the document to drawing
# its first page complete (ecclesia-canvas-win, `Canvas::draw`). The median of $Runs launches with the document.
$opens = @()
$memSamplesMb = @()
$log = Join-Path ([IO.Path]::GetTempPath()) "ecclesiapdf-budget-$PID.log"
$env:RUST_LOG = 'info,ecclesia_canvas_win=debug'
# Each launch gets a profile of its own, so every run is a first open and none restores the last run's session.
$saved = @{ APPDATA = $env:APPDATA; LOCALAPPDATA = $env:LOCALAPPDATA }
$profiles = Join-Path ([IO.Path]::GetTempPath()) "ecclesiapdf-budget-profiles-$PID"
try {
    for ($i = 0; $i -lt $Runs; $i++) {
        $env:APPDATA = Join-Path $profiles "$i/Roaming"
        $env:LOCALAPPDATA = Join-Path $profiles "$i/Local"
        New-Item -ItemType Directory -Force $env:APPDATA, $env:LOCALAPPDATA | Out-Null
        $p = Start-Process -FilePath $Exe -ArgumentList "`"$Document`"" -PassThru -RedirectStandardError $log
        $deadline = (Get-Date).AddSeconds(15)
        $ms = $null
        while ($null -eq $ms) {
            $line = Get-Content $log -ErrorAction SilentlyContinue | Select-String 'first page drawn ([\d.]+) ms' | Select-Object -First 1
            if ($line) { $ms = [double]$line.Matches[0].Groups[1].Value; break }
            if ($p.HasExited -or (Get-Date) -gt $deadline) {
                if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
                Write-Host "ecclesiapdf's log of run $($i + 1):"
                Get-Content $log -Tail 60 -ErrorAction SilentlyContinue | Write-Host
                throw "ecclesiapdf drew no first page of $Document within 15 s (run $($i + 1))"
            }
            Start-Sleep -Milliseconds 20
        }
        $opens += [math]::Round($ms, 1)
        Start-Sleep -Milliseconds 200
        $p.Refresh()
        if (-not $p.HasExited) {
            $memSamplesMb += [math]::Round($p.WorkingSet64 / 1MB, 1)
        }
        Stop-Process -Id $p.Id -Force
        $p.WaitForExit()
        Remove-Item $log
        Start-Sleep -Milliseconds 500
    }
} finally {
    Remove-Item Env:RUST_LOG
    $env:APPDATA = $saved.APPDATA
    $env:LOCALAPPDATA = $saved.LOCALAPPDATA
    Remove-Item -Recurse -Force $profiles -ErrorAction SilentlyContinue
}
$openMs = ($opens | Sort-Object)[[int][math]::Floor($Runs / 2)]
$idleMemMb = if ($memSamplesMb.Count -gt 0) {
    ($memSamplesMb | Sort-Object)[[int][math]::Floor($memSamplesMb.Count / 2)]
} else { 0 }

# A baseline is recorded per budget in its budgets.json ("baseline"), on the CI runner, and updated deliberately in the
# pull request that changes it. Without one, the measurement is only reported.
$rows = @(
    @{ Name = 'Core installer (exe as an LZX cabinet)'; Unit = 'MB'; Value = $installerMb; Entry = $size.budgets.core_installer_mb; Tolerance = $size.regression_tolerance_percent },
    @{ Name = "Cold start to usable window (median of $Runs)"; Unit = 'ms'; Value = $coldStartMs; Entry = $perf.budgets.cold_start_to_usable_window_ms; Tolerance = $perf.regression_tolerance_percent },
    @{ Name = "Open a typical PDF to first page (median of $Runs)"; Unit = 'ms'; Value = $openMs; Entry = $perf.budgets.open_typical_pdf_first_page_ms; Tolerance = $perf.regression_tolerance_percent },
    @{ Name = "Idle memory, one small document (median of $($memSamplesMb.Count))"; Unit = 'MB'; Value = $idleMemMb; Entry = $perf.budgets.idle_memory_one_small_document_mb; Tolerance = $perf.regression_tolerance_percent }
)
$failed = $false
$table = @('| Budget | Measured | Budget | Baseline | Result |', '| --- | ---: | ---: | ---: | --- |')
foreach ($r in $rows) {
    $budget = $r.Entry.budget
    $baseline = $r.Entry.baseline
    $result = if ($r.Value -le $budget) { 'within budget' } else { 'over budget' }
    if ($null -ne $baseline -and $r.Value -gt $baseline * (1 + $r.Tolerance / 100)) {
        $result = "regression (> $($r.Tolerance) % over baseline)"
        $failed = $true
    }
    $shownBaseline = if ($null -ne $baseline) { "$baseline $($r.Unit)" } else { 'none' }
    $table += "| $($r.Name) | $($r.Value) $($r.Unit) | $budget $($r.Unit) | $shownBaseline | $result |"
}
$table += ''
$table += "Cold start runs (ms): $($times -join ', ')"
$table += ''
$table += "First page runs (ms): $($opens -join ', ')"
$table += ''
$table += "Idle memory runs (MB): $($memSamplesMb -join ', ')"
$table -join "`n" | Write-Output
if ($env:GITHUB_STEP_SUMMARY) {
    @('## Budgets', '') + $table -join "`n" | Add-Content -Path $env:GITHUB_STEP_SUMMARY
}
if ($failed) { exit 1 }
