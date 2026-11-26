<#
.SYNOPSIS
Builds a test MSI into a temporary directory and validates its MSI 5.0 Single Package Authoring schema, UI dialogs,
size budget (<= 15 MB), and administrative unpack payload integrity.

.DESCRIPTION
Runs build-msi.ps1 into $OutDir (defaulting to a unique folder under $env:TEMP so dist/ releases are untouched),
inspects every Windows Installer SQL table and stream programmatically, and executes `msiexec /a` to verify all
4 payload files unpack cleanly and match their source SHA-256 hashes.

.EXAMPLE
pwsh packaging/windows/test-msi.ps1
#>
[CmdletBinding()]
param(
    [string] $Exe = 'target/release/excellentpdf.exe',
    [string] $OutDir = (Join-Path ([IO.Path]::GetTempPath()) "excellentpdf-msi-test-$PID"),
    [string] $CrtDir,
    [switch] $KeepTemp
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Resolve-Path (Join-Path $PSScriptRoot '../..')
$buildScript = Join-Path $PSScriptRoot 'build-msi.ps1'
$extractDir = Join-Path $OutDir 'admin-extract'

function ConvertTo-Plain([object[]] $arguments) {
    , [object[]] @(for ($i = 0; $i -lt $arguments.Count; $i++) {
            $value = $arguments[$i]
            if ($value -is [psobject]) { $value.psobject.BaseObject } else { $value }
        })
}
function Invoke-Com($object, [string] $member, [object[]] $arguments = @()) {
    $object.GetType().InvokeMember($member, 'InvokeMethod', $null, $object, (ConvertTo-Plain $arguments))
}
function Get-ComProperty($object, [string] $member, [object[]] $arguments = @()) {
    $object.GetType().InvokeMember($member, 'GetProperty', $null, $object, (ConvertTo-Plain $arguments))
}

$failures = New-Object System.Collections.Generic.List[string]
function Assert-True([bool] $condition, [string] $message) {
    if (-not $condition) { $failures.Add($message) }
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
try {
    $buildArgs = @{ Release = 0; Exe = $Exe; OutDir = $OutDir }
    if ($CrtDir) { $buildArgs.CrtDir = $CrtDir }

    Write-Host "Building test MSI in $OutDir ..."
    $built = & $buildScript @buildArgs
    $msiPath = ($built | Select-Object -Last 1).FullName
    if (-not $msiPath -or -not (Test-Path $msiPath)) { throw "build-msi.ps1 did not produce an MSI file" }
    $msiItem = Get-Item $msiPath
    $sizeMb = [math]::Round($msiItem.Length / 1MB, 2)
    Write-Host "Built: $($msiItem.Name) ($sizeMb MB)"

    # 1. Naming (DD3) and size budget (<= 15 MB)
    Assert-True ($msiItem.Name -match '^ExcellentPDF-v\d{2}-\d{2}-\d{2}\.0-v\d+\.\d+\.\d+-windows-x64\.msi$') "MSI name violates DD3: $($msiItem.Name)"
    Assert-True ($msiItem.Length -le (15 * 1MB)) "MSI size $sizeMb MB exceeds 15 MB budget"

    # 2. Open MSI read-only and validate every table and cross-reference
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $db = Invoke-Com $installer 'OpenDatabase' @($msiPath, 0) # msiOpenDatabaseModeReadOnly
    try {
        function Read-Table([string] $table, [string[]] $cols) {
            $colSql = ($cols | ForEach-Object { "``$_``" }) -join ', '
            $view = Invoke-Com $db 'OpenView' @("SELECT $colSql FROM ``$table``")
            [void] (Invoke-Com $view 'Execute')
            $rows = New-Object System.Collections.Generic.List[hashtable]
            while ($true) {
                $rec = Invoke-Com $view 'Fetch'
                if ($null -eq $rec) { break }
                $row = @{}
                for ($i = 0; $i -lt $cols.Count; $i++) {
                    $row[$cols[$i]] = Get-ComProperty $rec 'StringData' @(($i + 1))
                }
                $rows.Add($row)
            }
            [void] (Invoke-Com $view 'Close')
            return $rows
        }

        # Summary Information checks
        $summary = Get-ComProperty $db 'SummaryInformation' @(0)
        $pageCount = [int](Get-ComProperty $summary 'Property' @(14))
        $wordCount = [int](Get-ComProperty $summary 'Property' @(15))
        $template = [string](Get-ComProperty $summary 'Property' @(7))
        [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($summary)
        Assert-True ($pageCount -eq 500) "Expected MSI 5.0 Page Count 500, got $pageCount"
        Assert-True ($wordCount -eq 2) "Expected Word Count Summary 2 (compressed + UAC elevation enabled for All Users), got $wordCount"
        Assert-True ($template -eq 'x64;1033') "Expected Template 'x64;1033', got '$template'"

        # Property checks (Single Package Authoring)
        $props = @{}
        foreach ($r in (Read-Table 'Property' @('Property', 'Value'))) { $props[$r.Property] = $r.Value }
        Assert-True ($props['ALLUSERS'] -eq '2') "Expected ALLUSERS=2, got '$($props['ALLUSERS'])'"
        Assert-True ($props['MSIINSTALLPERUSER'] -eq '1') "Expected MSIINSTALLPERUSER=1, got '$($props['MSIINSTALLPERUSER'])'"
        Assert-True ($props['ARPPRODUCTICON'] -eq 'AppIcon.ico') "Expected ARPPRODUCTICON=AppIcon.ico"
        Assert-True ($props['ErrorDialog'] -eq 'ErrorDialog') "Expected ErrorDialog='ErrorDialog'"
        Assert-True ($props['DefaultUIFont'] -eq 'DlgFont8') "Expected DefaultUIFont='DlgFont8'"
        foreach ($sec in @('OLDERFOUND', 'NEWERFOUND', 'INSTALLDIR', 'ALLUSERS', 'MSIINSTALLPERUSER', 'USERCUSTOMDIR')) {
            Assert-True (($props['SecureCustomProperties'] -split ';') -contains $sec) "SecureCustomProperties missing $sec"
        }

        # Directory table: every Directory_Parent chain must terminate at TARGETDIR
        $dirs = @{}
        foreach ($r in (Read-Table 'Directory' @('Directory', 'Directory_Parent', 'DefaultDir'))) {
            $dirs[$r.Directory] = $r
        }
        Assert-True ($dirs.ContainsKey('TARGETDIR') -and $dirs['TARGETDIR'].Directory_Parent -eq '') "TARGETDIR missing or has parent"
        foreach ($d in $dirs.Keys) {
            $cur = $d; $hops = 0
            while ($cur -ne 'TARGETDIR') {
                $parent = $dirs[$cur].Directory_Parent
                Assert-True ($parent -ne '' -and $dirs.ContainsKey($parent)) "Directory '$d' has broken parent '$parent'"
                if ($parent -eq '' -or -not $dirs.ContainsKey($parent)) { break }
                $cur = $parent; $hops++
                if ($hops -gt 32) { Assert-True $false "Directory cycle at '$d'"; break }
            }
        }
        Assert-True ($dirs.ContainsKey('INSTALLDIR')) "Directory INSTALLDIR missing"
        Assert-True ($dirs.ContainsKey('ProgramFiles64Folder')) "Directory ProgramFiles64Folder missing"
        Assert-True ($dirs.ContainsKey('ProgramMenuFolder')) "Directory ProgramMenuFolder missing"

        # Registry table: Root must be -1 (HKCU/HKLM dynamic)
        $regs = @{}
        foreach ($r in (Read-Table 'Registry' @('Registry', 'Root', 'Key', 'Name', 'Value', 'Component_'))) {
            $regs[$r.Registry] = $r
            Assert-True ([int]$r.Root -eq -1) "Registry '$($r.Registry)' Root must be -1 for dual-scope, got $($r.Root)"
        }

        # Component table: Directory_ and KeyPath valid
        $filesTbl = @{}
        foreach ($r in (Read-Table 'File' @('File', 'Component_', 'FileName', 'FileSize'))) { $filesTbl[$r.File] = $r }
        Assert-True ($filesTbl.Count -eq 4) "Expected 4 files in File table, got $($filesTbl.Count)"
        foreach ($c in (Read-Table 'Component' @('Component', 'Directory_', 'Attributes', 'KeyPath'))) {
            Assert-True ($dirs.ContainsKey($c.Directory_)) "Component '$($c.Component)' references unknown Directory '$($c.Directory_)'"
            $attrs = [int]$c.Attributes
            if (($attrs -band 4) -ne 0) {
                Assert-True ($regs.ContainsKey($c.KeyPath)) "Component '$($c.Component)' registry KeyPath '$($c.KeyPath)' not in Registry table"
            } elseif ($c.KeyPath -ne '') {
                Assert-True ($filesTbl.ContainsKey($c.KeyPath)) "Component '$($c.Component)' file KeyPath '$($c.KeyPath)' not in File table"
            }
        }

        # Icon & Binary streams
        $icons = @{}
        foreach ($r in (Read-Table 'Icon' @('Name'))) { $icons[$r.Name] = $true }
        Assert-True ($icons.ContainsKey('AppIcon.ico')) "Icon stream AppIcon.ico missing"

        $binaries = @{}
        foreach ($r in (Read-Table 'Binary' @('Name'))) { $binaries[$r.Name] = $true }
        foreach ($b in @('AppIconBin', 'BannerSide', 'TopBanner', 'UpIco', 'NewFolderIco')) {
            Assert-True ($binaries.ContainsKey($b)) "Binary stream '$b' missing"
        }

        # Validate 24-bit BMP Binary streams (BannerSide 164x312, TopBanner 493x58)
        function Read-BinaryBytes([string] $binName) {
            $v = Invoke-Com $db 'OpenView' @("SELECT ``Data`` FROM ``Binary`` WHERE ``Name`` = '$binName'")
            [void] (Invoke-Com $v 'Execute')
            $rec = Invoke-Com $v 'Fetch'
            $sz = [int](Get-ComProperty $rec 'DataSize' @(1))
            $rawStr = Invoke-Com $rec 'ReadStream' @(1, $sz, 2) # msiReadStreamBytes = 2
            $bytes = [Text.Encoding]::Default.GetBytes($rawStr)
            [void] (Invoke-Com $v 'Close')
            return $bytes
        }
        $bannerChecks = @{ BannerSide = @(164, 312); TopBanner = @(493, 58) }
        foreach ($bn in $bannerChecks.Keys) {
            $bmp = Read-BinaryBytes $bn
            $expW = $bannerChecks[$bn][0]; $expH = $bannerChecks[$bn][1]
            $bmOk = ($bmp.Length -ge 54 -and $bmp[0] -eq 0x42 -and $bmp[1] -eq 0x4D)
            $bw = if ($bmOk) { [BitConverter]::ToInt32($bmp, 18) } else { 0 }
            $bh = if ($bmOk) { [BitConverter]::ToInt32($bmp, 22) } else { 0 }
            $bbpp = if ($bmOk) { [BitConverter]::ToUInt16($bmp, 28) } else { 0 }
            Assert-True ($bmOk -and $bw -eq $expW -and $bh -eq $expH -and $bbpp -eq 24) "Binary '$bn' must be ${expW}x${expH} 24-bit BMP (got ${bw}x${bh} ${bbpp}bpp)"
        }

        # TextStyle, CheckBox, RadioButton, UIText, ActionText, Error
        $styles = @{}
        foreach ($r in (Read-Table 'TextStyle' @('TextStyle'))) { $styles[$r.TextStyle] = $true }
        Assert-True ($styles.ContainsKey($props['DefaultUIFont'])) "DefaultUIFont '$($props['DefaultUIFont'])' missing from TextStyle"

        $radios = Read-Table 'RadioButton' @('Property', 'Order', 'Value', 'Text')
        $scopeRadios = @($radios | Where-Object { $_.Property -eq 'INSTALLSCOPE' })
        Assert-True ($scopeRadios.Count -eq 2) "Expected 2 INSTALLSCOPE radio buttons, got $($scopeRadios.Count)"
        Assert-True (($scopeRadios.Value -contains 'PerUser') -and ($scopeRadios.Value -contains 'PerMachine')) "INSTALLSCOPE radio buttons must include PerUser and PerMachine"

        $checkboxes = @{}
        foreach ($r in (Read-Table 'CheckBox' @('Property', 'Value'))) { $checkboxes[$r.Property] = $r.Value }
        Assert-True ($checkboxes['LAUNCHAPP'] -eq '1') "CheckBox LAUNCHAPP missing or != 1"

        $uiTextMap = @{}
        foreach ($r in (Read-Table 'UIText' @('Key', 'Text'))) { $uiTextMap[$r.Key] = $r.Text }
        Assert-True ($uiTextMap.ContainsKey('NewFolder')) "UIText missing NewFolder (required by DirectoryListNew)"

        # Dialog & Control cross-reference integrity
        $dialogs = @{}
        foreach ($r in (Read-Table 'Dialog' @('Dialog', 'Attributes', 'Control_First', 'Control_Default', 'Control_Cancel'))) {
            $dialogs[$r.Dialog] = $r
        }
        $requiredDialogs = @(
            'PrepareDialog', 'WelcomeDialog', 'ScopeDialog', 'FolderDialog', 'BrowseDialog',
            'VerifyReadyDialog', 'MaintenanceDialog', 'ProgressDialog', 'ExitDialog',
            'CancelDialog', 'UserExitDialog', 'FatalErrorDialog', 'ErrorDialog'
        )
        foreach ($d in $requiredDialogs) {
            Assert-True ($dialogs.ContainsKey($d)) "Required dialog '$d' missing from Dialog table"
        }
        Assert-True (([int]$dialogs['ErrorDialog'].Attributes -band 65536) -ne 0) "ErrorDialog missing Error attribute (65536)"
        Assert-True (([int]$dialogs['ProgressDialog'].Attributes -band 2) -eq 0) "ProgressDialog must be modeless (Modal bit clear)"
        Assert-True (([int]$dialogs['PrepareDialog'].Attributes -band 2) -eq 0) "PrepareDialog must be modeless (Modal bit clear)"

        $controlsByDialog = @{}
        foreach ($d in $dialogs.Keys) { $controlsByDialog[$d] = @{} }
        $hasElevationShield = $false
        $hasPathEdit = $false
        $hasDirCombo = $false
        $hasDirList = $false
        $hasProgressBar = $false
        foreach ($c in (Read-Table 'Control' @('Dialog_', 'Control', 'Type', 'Attributes', 'Property', 'Text', 'Control_Next'))) {
            Assert-True ($dialogs.ContainsKey($c.Dialog_)) "Control '$($c.Dialog_).$($c.Control)' references missing Dialog"
            if ($dialogs.ContainsKey($c.Dialog_)) { $controlsByDialog[$c.Dialog_][$c.Control] = $c }
            $attrs = [int]$c.Attributes
            if ($c.Dialog_ -eq 'VerifyReadyDialog' -and $c.Type -eq 'PushButton' -and (($attrs -band 8388608) -ne 0)) {
                $hasElevationShield = $true
            }
            if ($c.Dialog_ -eq 'FolderDialog' -and $c.Type -eq 'PathEdit' -and $c.Property -eq 'INSTALLDIR') { $hasPathEdit = $true }
            if ($c.Dialog_ -eq 'BrowseDialog' -and $c.Type -eq 'DirectoryCombo') { $hasDirCombo = $true }
            if ($c.Dialog_ -eq 'BrowseDialog' -and $c.Type -eq 'DirectoryList') { $hasDirList = $true }
            if ($c.Dialog_ -eq 'ProgressDialog' -and $c.Type -eq 'ProgressBar') { $hasProgressBar = $true }
            if ($c.Type -in @('Bitmap', 'Icon') -or ($c.Type -eq 'PushButton' -and (($attrs -band 524288) -ne 0))) {
                Assert-True ($binaries.ContainsKey($c.Text)) "Control '$($c.Dialog_).$($c.Control)' references missing Binary '$($c.Text)'"
            }
        }
        Assert-True $hasElevationShield "VerifyReadyDialog missing PushButton with ElevationShield (8388608)"
        Assert-True $hasPathEdit "FolderDialog missing PathEdit bound to INSTALLDIR"
        Assert-True ($hasDirCombo -and $hasDirList) "BrowseDialog missing DirectoryCombo / DirectoryList"
        Assert-True $hasProgressBar "ProgressDialog missing ProgressBar"

        # Verify Dialog.Control_First, Control_Default, Control_Cancel, and Control.Control_Next
        foreach ($d in $dialogs.Keys) {
            $dlg = $dialogs[$d]
            $ctls = $controlsByDialog[$d]
            Assert-True ($ctls.ContainsKey($dlg.Control_First)) "Dialog '$d' Control_First '$($dlg.Control_First)' does not exist"
            if ($dlg.Control_Default -ne '') {
                Assert-True ($ctls.ContainsKey($dlg.Control_Default)) "Dialog '$d' Control_Default '$($dlg.Control_Default)' does not exist"
            }
            if ($dlg.Control_Cancel -ne '') {
                Assert-True ($ctls.ContainsKey($dlg.Control_Cancel)) "Dialog '$d' Control_Cancel '$($dlg.Control_Cancel)' does not exist"
            }
            foreach ($cn in $ctls.Keys) {
                $nxt = $ctls[$cn].Control_Next
                if ($d -eq 'ErrorDialog' -and $cn -in @('O', 'C', 'Y', 'N', 'R', 'I', 'A')) {
                    Assert-True ($nxt -eq '') "ErrorDialog button '$cn' Control_Next must be empty to prevent MSI Error 2814 (got '$nxt')"
                } elseif ($nxt -ne '') {
                    Assert-True ($ctls.ContainsKey($nxt)) "Control '$d.$cn' Control_Next '$nxt' does not exist"
                }
            }
        }

        # Verify ControlEvent targets (dialogs, custom actions, standard browse events)
        $customActions = @{}
        foreach ($r in (Read-Table 'CustomAction' @('Action', 'Type'))) { $customActions[$r.Action] = [int]$r.Type }
        $browseEvents = @{}
        foreach ($ev in (Read-Table 'ControlEvent' @('Dialog_', 'Control_', 'Event', 'Argument', 'Condition'))) {
            Assert-True ($dialogs.ContainsKey($ev.Dialog_) -and $controlsByDialog[$ev.Dialog_].ContainsKey($ev.Control_)) "ControlEvent references missing control '$($ev.Dialog_).$($ev.Control_)'"
            if ($ev.Dialog_ -eq 'ScopeDialog' -and $ev.Event -eq '[ALLUSERS]') {
                Assert-True ($ev.Argument -eq '2') "ScopeDialog must keep ALLUSERS=2 (got '$($ev.Argument)' for '$($ev.Condition)')"
            }
            if ($ev.Event -in @('NewDialog', 'SpawnDialog')) {
                Assert-True ($dialogs.ContainsKey($ev.Argument)) "ControlEvent $($ev.Event) on '$($ev.Dialog_).$($ev.Control_)' references missing Dialog '$($ev.Argument)'"
            }
            if ($ev.Event -eq 'DoAction') {
                Assert-True ($customActions.ContainsKey($ev.Argument)) "ControlEvent DoAction on '$($ev.Dialog_).$($ev.Control_)' references missing CustomAction '$($ev.Argument)'"
            }
            if ($ev.Dialog_ -eq 'BrowseDialog') { $browseEvents[$ev.Event] = $true }
        }
        foreach ($reqEv in @('DirectoryListUp', 'DirectoryListNew', 'SetTargetPath')) {
            Assert-True ($browseEvents.ContainsKey($reqEv)) "BrowseDialog missing standard ControlEvent '$reqEv'"
        }

        # Verify ControlCondition and EventMapping targets
        foreach ($cc in (Read-Table 'ControlCondition' @('Dialog_', 'Control_', 'Action'))) {
            Assert-True ($dialogs.ContainsKey($cc.Dialog_) -and $controlsByDialog[$cc.Dialog_].ContainsKey($cc.Control_)) "ControlCondition references missing control '$($cc.Dialog_).$($cc.Control_)'"
        }
        $mappings = @{}
        foreach ($em in (Read-Table 'EventMapping' @('Dialog_', 'Control_', 'Event', 'Attribute'))) {
            Assert-True ($dialogs.ContainsKey($em.Dialog_) -and $controlsByDialog[$em.Dialog_].ContainsKey($em.Control_)) "EventMapping references missing control '$($em.Dialog_).$($em.Control_)'"
            $mappings["$($em.Dialog_):$($em.Event)"] = $em.Attribute
        }
        Assert-True ($mappings['ProgressDialog:ActionText'] -eq 'Text') "ProgressDialog missing ActionText EventMapping"
        Assert-True ($mappings['ProgressDialog:SetProgress'] -eq 'Progress') "ProgressDialog missing SetProgress EventMapping"

        # Verify InstallUISequence dialog and custom action references
        $standardActions = @(
            'FindRelatedProducts', 'LaunchConditions', 'ValidateProductID', 'CostInitialize',
            'FileCost', 'CostFinalize', 'ExecuteAction'
        )
        $uiSeq = @{}
        foreach ($r in (Read-Table 'InstallUISequence' @('Action', 'Condition', 'Sequence'))) {
            $uiSeq[$r.Action] = [int]$r.Sequence
            $known = ($standardActions -contains $r.Action) -or $dialogs.ContainsKey($r.Action) -or $customActions.ContainsKey($r.Action)
            Assert-True $known "InstallUISequence references unknown action/dialog '$($r.Action)'"
        }
        Assert-True ($uiSeq['ExitDialog'] -eq -1) "InstallUISequence ExitDialog must have Sequence -1"
        Assert-True ($uiSeq['UserExitDialog'] -eq -2) "InstallUISequence UserExitDialog must have Sequence -2"
        Assert-True ($uiSeq['FatalErrorDialog'] -eq -3) "InstallUISequence FatalErrorDialog must have Sequence -3"
        Assert-True ($uiSeq.ContainsKey('WelcomeDialog') -and $uiSeq.ContainsKey('MaintenanceDialog') -and $uiSeq.ContainsKey('ProgressDialog')) "InstallUISequence missing WelcomeDialog / MaintenanceDialog / ProgressDialog"

        Write-Host "MSI table & schema validation complete ($($dialogs.Count) dialogs, $($filesTbl.Count) files, $($binaries.Count) binaries)."
    }
    finally {
        [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($db)
        [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
        [GC]::Collect()
        [GC]::WaitForPendingFinalizers()
    }

    # 3. Administrative unpack test (msiexec /a) + SHA-256 verification of all 4 payload files
    New-Item -ItemType Directory -Force $extractDir | Out-Null
    $logPath = Join-Path $OutDir 'admin-extract.log'
    Write-Host "Running msiexec /a unpack test into $extractDir ..."
    $proc = Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/a', "`"$msiPath`"", '/qn', "TARGETDIR=`"$extractDir`"", '/l*v', "`"$logPath`"") -Wait -PassThru
    Assert-True ($proc.ExitCode -eq 0) "msiexec /a exited with code $($proc.ExitCode) (see $logPath)"

    $exeFull = (Resolve-Path (Join-Path $root $Exe)).Path
    if (-not $CrtDir) {
        $CrtDir = Get-ChildItem -Directory -ErrorAction SilentlyContinue -Path @(
            "${env:ProgramFiles}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT",
            "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT"
        ) | Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
    }
    $expectedSources = @{
        'excellentpdf.exe' = $exeFull
        'msvcp140.dll'     = (Join-Path $CrtDir 'msvcp140.dll')
        'vcruntime140.dll' = (Join-Path $CrtDir 'vcruntime140.dll')
        'vcruntime140_1.dll' = (Join-Path $CrtDir 'vcruntime140_1.dll')
    }
    $extractedFiles = Get-ChildItem -Path $extractDir -Recurse -File
    foreach ($name in $expectedSources.Keys) {
        $match = $extractedFiles | Where-Object { $_.Name -ieq $name } | Select-Object -First 1
        Assert-True ($null -ne $match) "Unpacked payload missing '$name'"
        if ($null -ne $match) {
            $srcHash = (Get-FileHash -Path $expectedSources[$name] -Algorithm SHA256).Hash
            $dstHash = (Get-FileHash -Path $match.FullName -Algorithm SHA256).Hash
            Assert-True ($srcHash -eq $dstHash) "SHA-256 mismatch for '$name' (expected $srcHash, got $dstHash)"
        }
    }

    if ($failures.Count -gt 0) {
        Write-Host "`nFAILED ($($failures.Count) issues):" -ForegroundColor Red
        foreach ($f in $failures) { Write-Host "  - $f" -ForegroundColor Red }
        exit 1
    }

    Write-Host "`nALL CHECKS PASSED: $($msiItem.Name) ($sizeMb MB <= 15 MB), all 13 dialogs & dual-scope tables valid, msiexec /a exit 0 with all 4 payload files byte-identical." -ForegroundColor Green
}
finally {
    if (-not $KeepTemp) {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $OutDir
    }
}
