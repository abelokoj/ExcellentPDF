<#
.SYNOPSIS
Builds ExcellentPDF's per-user Windows installer (plan K.25) from a release build.

.DESCRIPTION
Writes an MSI with the Windows Installer API and makecab, both part of Windows, so no installer toolkit is needed.
The installer:
  - installs for the current user only, into %LOCALAPPDATA%\Programs\ExcellentPDF, without asking for administrator
    rights;
  - adds a Start menu shortcut, and the app to Settings > Apps, from where it uninstalls;
  - replaces any installed build of the same or an older version, and refuses to replace a newer one;
  - ships the Visual C++ runtime next to the app (app-local), so a fresh Windows needs nothing else;
  - starts the app when an install run with its window (not a silent one) finishes.
Uninstalling keeps the reader's settings and library.

The file is named as dev_decisions.md DD3 says: ExcellentPDF-vYY-MM-DD.N-vSEMVER-windows-x64.msi.

shortcut: no signing, no dialogs (Windows shows its own progress only) and no ARM64 build; K.25 adds them with the
release workflow.

.PARAMETER Release
N in the file name: the count of releases ever published, starting at 1.

.EXAMPLE
pwsh packaging/windows/build-msi.ps1 -Release 1
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [int] $Release,
    [string] $Exe = 'target/release/excellentpdf.exe',
    [string] $OutDir = 'dist',
    [string] $Date = (Get-Date -Format 'yy-MM-dd'),
    # The VC++ runtime folder (Microsoft.VC143.CRT, x64); found under Visual Studio when not given.
    [string] $CrtDir
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Fixed for the product's life: Windows matches upgrades and components by them.
$UpgradeCode = '{8FCE077C-FCA0-41E4-8764-CA608322B541}'
$AppComponentId = '{71AE9D01-4A64-4D89-8A9E-FDE53EF2391E}'
$ShortcutComponentId = '{AD14514A-6A5E-46D0-9CCF-CC3F8904E567}'

$root = Resolve-Path (Join-Path $PSScriptRoot '../..')
Push-Location $root
try {
    $Exe = (Resolve-Path $Exe).Path
    $version = (Select-String -Path Cargo.toml -Pattern '^version = "(\d+\.\d+\.\d+)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
    if (-not $CrtDir) {
        $CrtDir = Get-ChildItem -Directory -ErrorAction SilentlyContinue -Path @(
            "${env:ProgramFiles}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT",
            "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT"
        ) | Sort-Object FullName | Select-Object -Last 1 -ExpandProperty FullName
        if (-not $CrtDir) { throw 'No Visual C++ runtime (Microsoft.VC14*.CRT, x64) found: pass -CrtDir' }
    }
    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $OutDir = (Resolve-Path $OutDir).Path
    $name = "ExcellentPDF-v$Date.$Release-v$version-windows-x64"
    $msi = Join-Path $OutDir "$name.msi"
}
finally { Pop-Location }

# The installed files: key (the name inside the cabinet), MSI file name (short|long), source.
$files = @(
    @{ Key = 'excellentpdf.exe'; Name = 'EXCELL~1.EXE|excellentpdf.exe'; Path = $Exe }
    @{ Key = 'msvcp140.dll'; Name = 'msvcp140.dll'; Path = Join-Path $CrtDir 'msvcp140.dll' }
    @{ Key = 'vcruntime140.dll'; Name = 'VCRUNT~1.DLL|vcruntime140.dll'; Path = Join-Path $CrtDir 'vcruntime140.dll' }
    @{ Key = 'vcruntime140_1.dll'; Name = 'VCRUNT~2.DLL|vcruntime140_1.dll'; Path = Join-Path $CrtDir 'vcruntime140_1.dll' }
)

# The cabinet, LZX:21 as plan §13.1's size baseline measures it.
$work = Join-Path ([IO.Path]::GetTempPath()) "excellentpdf-msi-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $ddf = @(
        '.OPTION EXPLICIT', '.Set CabinetNameTemplate=product.cab', ".Set DiskDirectoryTemplate=$work",
        '.Set CompressionType=LZX', '.Set CompressionMemory=21', '.Set Cabinet=on', '.Set Compress=on',
        '.Set MaxDiskSize=0', '.Set RptFileName=nul', '.Set InfFileName=nul'
    ) + ($files | ForEach-Object { "`"$($_.Path)`" `"$($_.Key)`"" })
    $ddfPath = Join-Path $work 'product.ddf'
    Set-Content -Path $ddfPath -Value $ddf -Encoding ascii
    & makecab.exe /F $ddfPath | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "makecab failed ($LASTEXITCODE)" }
    $cab = Join-Path $work 'product.cab'

    # Windows Installer's automation objects have no type information PowerShell can bind to, and take plain values,
    # not PowerShell's wrappers.
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
    function Set-ComProperty($object, [string] $member, [object[]] $arguments) {
        [void] $object.GetType().InvokeMember($member, 'SetProperty', $null, $object, (ConvertTo-Plain $arguments))
    }

    $installer = New-Object -ComObject WindowsInstaller.Installer
    Remove-Item -Force -ErrorAction SilentlyContinue $msi
    $db = Invoke-Com $installer 'OpenDatabase' @($msi, 3) # msiOpenDatabaseModeCreate

    function Invoke-Sql([string] $sql, $record = $null) {
        $view = Invoke-Com $db 'OpenView' @($sql)
        if ($null -eq $record) { [void] (Invoke-Com $view 'Execute') } else { [void] (Invoke-Com $view 'Execute' @($record)) }
        [void] (Invoke-Com $view 'Close')
    }

    # The tables, from the Windows Installer database reference; columns as MSI SQL declares them.
    $tables = [ordered] @{
        Property               = '`Property` CHAR(72) NOT NULL, `Value` LONGCHAR NOT NULL LOCALIZABLE PRIMARY KEY `Property`'
        Directory              = '`Directory` CHAR(72) NOT NULL, `Directory_Parent` CHAR(72), `DefaultDir` CHAR(255) NOT NULL LOCALIZABLE PRIMARY KEY `Directory`'
        Component              = '`Component` CHAR(72) NOT NULL, `ComponentId` CHAR(38), `Directory_` CHAR(72) NOT NULL, `Attributes` SHORT NOT NULL, `Condition` CHAR(255), `KeyPath` CHAR(72) PRIMARY KEY `Component`'
        Feature                = '`Feature` CHAR(38) NOT NULL, `Feature_Parent` CHAR(38), `Title` CHAR(64) LOCALIZABLE, `Description` CHAR(255) LOCALIZABLE, `Display` SHORT, `Level` SHORT NOT NULL, `Directory_` CHAR(72), `Attributes` SHORT NOT NULL PRIMARY KEY `Feature`'
        FeatureComponents      = '`Feature_` CHAR(38) NOT NULL, `Component_` CHAR(72) NOT NULL PRIMARY KEY `Feature_`, `Component_`'
        File                   = '`File` CHAR(72) NOT NULL, `Component_` CHAR(72) NOT NULL, `FileName` CHAR(255) NOT NULL LOCALIZABLE, `FileSize` LONG NOT NULL, `Version` CHAR(72), `Language` CHAR(20), `Attributes` SHORT, `Sequence` SHORT NOT NULL PRIMARY KEY `File`'
        Media                  = '`DiskId` SHORT NOT NULL, `LastSequence` SHORT NOT NULL, `DiskPrompt` CHAR(64) LOCALIZABLE, `Cabinet` CHAR(255), `VolumeLabel` CHAR(32), `Source` CHAR(72) PRIMARY KEY `DiskId`'
        Registry               = '`Registry` CHAR(72) NOT NULL, `Root` SHORT NOT NULL, `Key` CHAR(255) NOT NULL LOCALIZABLE, `Name` CHAR(255) LOCALIZABLE, `Value` LONGCHAR LOCALIZABLE, `Component_` CHAR(72) NOT NULL PRIMARY KEY `Registry`'
        RemoveFile             = '`FileKey` CHAR(72) NOT NULL, `Component_` CHAR(72) NOT NULL, `FileName` CHAR(255) LOCALIZABLE, `DirProperty` CHAR(72) NOT NULL, `InstallMode` SHORT NOT NULL PRIMARY KEY `FileKey`'
        Shortcut               = '`Shortcut` CHAR(72) NOT NULL, `Directory_` CHAR(72) NOT NULL, `Name` CHAR(128) NOT NULL LOCALIZABLE, `Component_` CHAR(72) NOT NULL, `Target` CHAR(72) NOT NULL, `Arguments` CHAR(255), `Description` CHAR(255) LOCALIZABLE, `Hotkey` SHORT, `Icon_` CHAR(72), `IconIndex` SHORT, `ShowCmd` SHORT, `WkDir` CHAR(72) PRIMARY KEY `Shortcut`'
        Upgrade                = '`UpgradeCode` CHAR(38) NOT NULL, `VersionMin` CHAR(20), `VersionMax` CHAR(20), `Language` CHAR(255), `Attributes` LONG NOT NULL, `Remove` CHAR(255), `ActionProperty` CHAR(72) NOT NULL PRIMARY KEY `UpgradeCode`, `VersionMin`, `VersionMax`, `Language`, `Attributes`'
        LaunchCondition        = '`Condition` CHAR(255) NOT NULL, `Description` CHAR(255) NOT NULL LOCALIZABLE PRIMARY KEY `Condition`'
        CustomAction           = '`Action` CHAR(72) NOT NULL, `Type` SHORT NOT NULL, `Source` CHAR(72), `Target` CHAR(255) PRIMARY KEY `Action`'
        InstallExecuteSequence = '`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT PRIMARY KEY `Action`'
        InstallUISequence      = '`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT PRIMARY KEY `Action`'
        AdminExecuteSequence   = '`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT PRIMARY KEY `Action`'
        AdminUISequence        = '`Action` CHAR(72) NOT NULL, `Condition` CHAR(255), `Sequence` SHORT PRIMARY KEY `Action`'
    }
    foreach ($table in $tables.Keys) { Invoke-Sql "CREATE TABLE ``$table`` ($($tables[$table]))" }

    # Adds one row; $null leaves a column empty.
    function Add-Row([string] $table, [object[]] $values) {
        $columns = [regex]::Matches(($tables[$table] -split 'PRIMARY KEY')[0], '`([^`]+)`') | ForEach-Object { $_.Groups[1].Value }
        if ($columns.Count -ne $values.Count) { throw "$table takes $($columns.Count) values, got $($values.Count)" }
        $record = Invoke-Com $installer 'CreateRecord' @($values.Count)
        for ($i = 0; $i -lt $values.Count; $i++) {
            $value = $values[$i]
            if ($null -eq $value) { continue }
            if ($value -is [int]) { Set-ComProperty $record 'IntegerData' @(($i + 1), $value) }
            else { Set-ComProperty $record 'StringData' @(($i + 1), [string] $value) }
        }
        $list = ($columns | ForEach-Object { "``$_``" }) -join ', '
        $marks = (@('?') * $values.Count) -join ', '
        Invoke-Sql "INSERT INTO ``$table`` ($list) VALUES ($marks)" $record
    }

    $productCode = "{$([guid]::NewGuid().ToString().ToUpper())}"
    $properties = [ordered] @{
        ProductCode            = $productCode
        ProductName            = 'ExcellentPDF'
        ProductVersion         = $version
        ProductLanguage        = '1033'
        Manufacturer           = 'ExcellentPDF'
        UpgradeCode            = $UpgradeCode
        ARPNOMODIFY            = '1'
        SecureCustomProperties = 'OLDERFOUND;NEWERFOUND'
    }
    foreach ($key in $properties.Keys) { Add-Row Property @($key, $properties[$key]) }

    Add-Row Directory @('TARGETDIR', $null, 'SourceDir')
    Add-Row Directory @('LocalAppDataFolder', 'TARGETDIR', '.')
    Add-Row Directory @('UserPrograms', 'LocalAppDataFolder', 'Programs')
    Add-Row Directory @('INSTALLDIR', 'UserPrograms', 'EXCELL~1|ExcellentPDF')
    Add-Row Directory @('ProgramMenuFolder', 'TARGETDIR', '.')

    # 4: the key path is a registry value (HKCU, as a per-user install's must be); 256: 64-bit.
    Add-Row Component @('App', $AppComponentId, 'INSTALLDIR', 260, $null, 'AppInstalled')
    Add-Row Component @('StartMenu', $ShortcutComponentId, 'ProgramMenuFolder', 260, $null, 'ShortcutInstalled')
    Add-Row Feature @('Main', $null, 'ExcellentPDF', 'The ExcellentPDF reader', 1, 1, 'INSTALLDIR', 0)
    Add-Row FeatureComponents @('Main', 'App')
    Add-Row FeatureComponents @('Main', 'StartMenu')

    $sequence = 0
    foreach ($file in $files) {
        $sequence++
        $info = Get-Item $file.Path
        $v = $info.VersionInfo
        $fileVersion = if ($v.FileMajorPart + $v.FileMinorPart + $v.FileBuildPart + $v.FilePrivatePart -gt 0) {
            "$($v.FileMajorPart).$($v.FileMinorPart).$($v.FileBuildPart).$($v.FilePrivatePart)"
        } else { $null }
        # 512: vital, the install fails rather than skip it.
        Add-Row File @($file.Key, 'App', $file.Name, [int] $info.Length, $fileVersion, $null, 512, $sequence)
    }
    Add-Row Media @(1, $sequence, $null, '#product.cab', $null, $null)

    $key = 'Software\ExcellentPDF\Installer'
    Add-Row Registry @('AppInstalled', 1, $key, 'InstallDir', '[INSTALLDIR]', 'App')
    Add-Row Registry @('ShortcutInstalled', 1, $key, 'StartMenuShortcut', '#1', 'StartMenu')
    Add-Row Registry @('InstallerKey', 1, $key, '-', $null, 'App')
    Add-Row RemoveFile @('RemoveInstallDir', 'App', $null, 'INSTALLDIR', 2)
    Add-Row Shortcut @('StartMenu', 'ProgramMenuFolder', 'EXCELL~1|ExcellentPDF', 'StartMenu', '[#excellentpdf.exe]',
        $null, 'Read, annotate and organise PDFs', $null, $null, $null, $null, 'INSTALLDIR')

    # Same or older builds are replaced (512: up to this version inclusive); a newer one stops the install (2: detect only).
    Add-Row Upgrade @($UpgradeCode, $null, $version, $null, 512, $null, 'OLDERFOUND')
    Add-Row Upgrade @($UpgradeCode, $version, $null, $null, 2, $null, 'NEWERFOUND')
    Add-Row LaunchCondition @('NOT NEWERFOUND', 'A newer version of ExcellentPDF is already installed.')
    # 34: run an installed exe; 192: without waiting for it.
    Add-Row CustomAction @('LaunchApp', 226, 'INSTALLDIR', '"[#excellentpdf.exe]"')

    $execute = [ordered] @{
        FindRelatedProducts = 25; LaunchConditions = 100; ValidateProductID = 700; CostInitialize = 800
        FileCost = 900; CostFinalize = 1000; InstallValidate = 1400; InstallInitialize = 1500
        RemoveExistingProducts = 1501; ProcessComponents = 1600; UnpublishFeatures = 1800
        RemoveRegistryValues = 2600; RemoveShortcuts = 3200; RemoveFiles = 3500; InstallFiles = 4000
        CreateShortcuts = 4500; WriteRegistryValues = 5000; RegisterUser = 6000; RegisterProduct = 6100
        PublishFeatures = 6300; PublishProduct = 6400; InstallFinalize = 6600
    }
    foreach ($action in $execute.Keys) { Add-Row InstallExecuteSequence @($action, $null, $execute[$action]) }
    # Started from its window (UILevel 5), not silently, and not when removing.
    Add-Row InstallExecuteSequence @('LaunchApp', 'UILevel > 3 AND NOT REMOVE', 6700)
    $ui = [ordered] @{
        FindRelatedProducts = 25; LaunchConditions = 100; ValidateProductID = 700; CostInitialize = 800
        FileCost = 900; CostFinalize = 1000; ExecuteAction = 1300
    }
    foreach ($action in $ui.Keys) { Add-Row InstallUISequence @($action, $null, $ui[$action]) }
    # An administrative install (msiexec /a) unpacks the files: how the build checks the package.
    $admin = [ordered] @{
        CostInitialize = 800; FileCost = 900; CostFinalize = 1000; InstallValidate = 1400
        InstallInitialize = 1500; InstallAdminPackage = 3900; InstallFiles = 4000; InstallFinalize = 6600
    }
    foreach ($action in $admin.Keys) { Add-Row AdminExecuteSequence @($action, $null, $admin[$action]) }
    foreach ($action in 'CostInitialize', 'FileCost', 'CostFinalize', 'ExecuteAction') {
        Add-Row AdminUISequence @($action, $null, $ui[$action])
    }

    $streams = Invoke-Com $db 'OpenView' @('SELECT `Name`, `Data` FROM `_Streams`')
    $record = Invoke-Com $installer 'CreateRecord' @(2)
    Set-ComProperty $record 'StringData' @(1, 'product.cab')
    [void] (Invoke-Com $record 'SetStream' @(2, $cab))
    [void] (Invoke-Com $streams 'Modify' @(1, $record)) # msiViewModifyInsert
    [void] (Invoke-Com $streams 'Close')

    $summary = Get-ComProperty $db 'SummaryInformation' @(20)
    $fields = [ordered] @{
        1  = [int] 1252                       # codepage
        2  = 'Installation Database'
        3  = 'ExcellentPDF'
        4  = 'ExcellentPDF'
        5  = 'Installer'
        7  = 'x64;1033'                       # platform and language
        9  = "{$([guid]::NewGuid().ToString().ToUpper())}" # package code, new for every build
        12 = [datetime]::Now
        13 = [datetime]::Now
        14 = [int] 500                        # Windows Installer 5.0: needed for x64
        15 = [int] 10                         # 2: compressed files; 8: no administrator rights needed
        18 = 'ExcellentPDF build script'
    }
    foreach ($field in $fields.GetEnumerator()) { Set-ComProperty $summary 'Property' @([int] $field.Key, $field.Value) }
    [void] (Invoke-Com $summary 'Persist')
    [void] (Invoke-Com $db 'Commit')
    [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($db)
    [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}

Get-Item $msi | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }, FullName
