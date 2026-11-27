<#
.SYNOPSIS
Builds EcclesiaPDF's dual-purpose (per-user / per-machine) Windows installer (MSI 5.0) from a release build.

.DESCRIPTION
Writes a self-contained Windows Installer 5.0 database using the Windows Installer COM API and makecab.exe, both
built into Windows, so no external installer toolkit is required on developer machines or CI runners.

Implements Windows Installer 5.0 Single Package Authoring (ALLUSERS=2, MSIINSTALLPERUSER=1 default, Word Count
Summary=10, Registry Root=-1) with a native interactive wizard UI:
  - WelcomeDialog: branded intro with the Nib & Leaf icon.
  - ScopeDialog: choose "Install just for me (Current User)" (default, %LOCALAPPDATA%\Programs\EcclesiaPDF, no UAC)
    or "Install for all users on this computer (Administrator)" (%ProgramFiles%\EcclesiaPDF, triggers UAC elevation).
  - FolderDialog + BrowseDialog: edit or browse to any target directory (DirectoryCombo, DirectoryList, PathEdit,
    Up, NewFolder, SetTargetPath) and opt in/out of registering supported file types.
  - VerifyReadyDialog: summarizes scope, target directory, and file associations; shows a UAC ElevationShield on the
    Install button when per-machine is selected.
  - ProgressDialog: live ActionText / ActionData / SetProgress bar with Cancel.
  - ExitDialog: completion screen with optional "Launch Ecclesia PDF" and "Open Windows Default Apps settings"
    checkboxes (fresh install only, runs unelevated).
  - MaintenanceDialog: Repair or Remove when EcclesiaPDF is already installed.

Also supports silent and scripted deployments:
  - Per-user default:  msiexec /i EcclesiaPDF-....msi /qn
  - Per-machine:       msiexec /i EcclesiaPDF-....msi ALLUSERS=1 MSIINSTALLPERUSER="" /qn
  - Custom directory:  msiexec /i EcclesiaPDF-....msi INSTALLDIR="C:\Custom\EcclesiaPDF" /qn
  - Skip file assoc:   msiexec /i EcclesiaPDF-....msi REGISTERFILETYPES="" /qn
  - Combined:          msiexec /i EcclesiaPDF-....msi ALLUSERS=1 MSIINSTALLPERUSER="" INSTALLDIR="C:\Apps\EcclesiaPDF" /qn

Preserves DD3 release naming (EcclesiaPDF-vYY-MM-DD.N-vSEMVER-windows-x64.msi), DD28 ARPPRODUCTICON, Start Menu
shortcut (resolves to User or Common Start Menu based on ALLUSERS), UpgradeCode upgrade/downgrade guards, app-local
Visual C++ runtime DLLs, and uninstall preservation of user settings and library data.

.PARAMETER Release
N in the file name: the count of releases ever published, starting at 1 (use 0 for test builds).

.EXAMPLE
pwsh packaging/windows/build-msi.ps1 -Release 1
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [int] $Release,
    [string] $Exe = 'target/release/ecclesiapdf.exe',
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
$FileAssocComponentId = '{E2C4F6A8-5B31-4D92-9A7E-6C1D8F4B2A09}'

# Pure .NET binary generators (zero System.Drawing dependency; works in both Windows PowerShell 5.1 and pwsh 7+).
function Write-UInt16LE([byte[]] $buf, [int] $offset, [int] $value) {
    $buf[$offset] = [byte]($value -band 0xFF)
    $buf[$offset + 1] = [byte](($value -shr 8) -band 0xFF)
}
function Write-Int32LE([byte[]] $buf, [int] $offset, [int] $value) {
    $buf[$offset] = [byte]($value -band 0xFF)
    $buf[$offset + 1] = [byte](($value -shr 8) -band 0xFF)
    $buf[$offset + 2] = [byte](($value -shr 16) -band 0xFF)
    $buf[$offset + 3] = [byte](($value -shr 24) -band 0xFF)
}

# Writes an 8-bit paletted bottom-up BMP (small, crisp, universally supported by MSI's GDI Bitmap control).
# $palette is an array of @(R, G, B) triples (up to 256); $pixels is top-down [byte[]] of length $width * $height.
function Write-IndexedBmp([string] $path, [int] $width, [int] $height, [object[]] $palette, [byte[]] $pixels) {
    $stride = ($width + 3) -band (-bnot 3)
    $imageSize = $stride * $height
    $headerSize = 14 + 40 + (256 * 4)
    $fileSize = $headerSize + $imageSize
    $buf = New-Object byte[] $fileSize

    # BITMAPFILEHEADER (14 bytes)
    $buf[0] = 0x42; $buf[1] = 0x4D # 'BM'
    Write-Int32LE $buf 2 $fileSize
    Write-Int32LE $buf 10 $headerSize

    # BITMAPINFOHEADER (40 bytes)
    Write-Int32LE $buf 14 40
    Write-Int32LE $buf 18 $width
    Write-Int32LE $buf 22 $height # positive = bottom-up DIB
    Write-UInt16LE $buf 26 1     # planes
    Write-UInt16LE $buf 28 8     # 8 bpp indexed
    Write-Int32LE $buf 34 $imageSize
    Write-Int32LE $buf 38 3780   # 96 DPI X
    Write-Int32LE $buf 42 3780   # 96 DPI Y
    Write-Int32LE $buf 46 256    # clrUsed

    # Palette (BGRA)
    for ($i = 0; $i -lt $palette.Count; $i++) {
        $p = $palette[$i]
        $off = 54 + ($i * 4)
        $buf[$off] = [byte]$p[2]     # B
        $buf[$off + 1] = [byte]$p[1] # G
        $buf[$off + 2] = [byte]$p[0] # R
        $buf[$off + 3] = 0
    }

    # Pixel rows (bottom-up)
    for ($y = 0; $y -lt $height; $y++) {
        $srcRow = ($height - 1 - $y) * $width
        $dstRow = $headerSize + ($y * $stride)
        [Array]::Copy($pixels, $srcRow, $buf, $dstRow, $width)
    }

    [IO.File]::WriteAllBytes($path, $buf)
}

# Writes an uncompressed 24-bit BGR bottom-up BMP (universally supported by MSI's GDI Bitmap control).
# $rgb is top-down [byte[]] of length $width * $height * 3 stored as RGB triples.
function Write-Rgb24Bmp([string] $path, [int] $width, [int] $height, [byte[]] $rgb) {
    $stride = ($width * 3 + 3) -band (-bnot 3)
    $imageSize = $stride * $height
    $headerSize = 14 + 40
    $fileSize = $headerSize + $imageSize
    $buf = New-Object byte[] $fileSize

    # BITMAPFILEHEADER (14 bytes)
    $buf[0] = 0x42; $buf[1] = 0x4D # 'BM'
    Write-Int32LE $buf 2 $fileSize
    Write-Int32LE $buf 10 $headerSize

    # BITMAPINFOHEADER (40 bytes)
    Write-Int32LE $buf 14 40
    Write-Int32LE $buf 18 $width
    Write-Int32LE $buf 22 $height # positive = bottom-up DIB
    Write-UInt16LE $buf 26 1     # planes
    Write-UInt16LE $buf 28 24    # 24 bpp BGR
    Write-Int32LE $buf 34 $imageSize
    Write-Int32LE $buf 38 3780   # 96 DPI X
    Write-Int32LE $buf 42 3780   # 96 DPI Y

    for ($y = 0; $y -lt $height; $y++) {
        $srcRow = ($height - 1 - $y) * $width * 3
        $dstRow = $headerSize + ($y * $stride)
        for ($x = 0; $x -lt $width; $x++) {
            $s = $srcRow + ($x * 3)
            $d = $dstRow + ($x * 3)
            $buf[$d] = $rgb[$s + 2]     # B
            $buf[$d + 1] = $rgb[$s + 1] # G
            $buf[$d + 2] = $rgb[$s]     # R
        }
    }

    [IO.File]::WriteAllBytes($path, $buf)
}

# Pure-.NET PNG decoder (DeflateStream + PNG scanline unfiltering) returning @{ Width; Height; Rgba }.
function Read-PngRgba([byte[]] $pngBytes) {
    if ($null -eq $pngBytes -or $pngBytes.Length -lt 33 -or $pngBytes[0] -ne 0x89 -or $pngBytes[1] -ne 0x50) {
        return $null
    }
    $w = 0; $h = 0; $colorType = 6; $bitDepth = 8
    $idat = New-Object IO.MemoryStream
    $pos = 8
    while ($pos + 12 -le $pngBytes.Length) {
        $len = ([int]$pngBytes[$pos] -shl 24) -bor ([int]$pngBytes[$pos + 1] -shl 16) -bor ([int]$pngBytes[$pos + 2] -shl 8) -bor [int]$pngBytes[$pos + 3]
        $type = [Text.Encoding]::ASCII.GetString($pngBytes, $pos + 4, 4)
        $dataOff = $pos + 8
        if ($type -eq 'IHDR') {
            $w = ([int]$pngBytes[$dataOff] -shl 24) -bor ([int]$pngBytes[$dataOff + 1] -shl 16) -bor ([int]$pngBytes[$dataOff + 2] -shl 8) -bor [int]$pngBytes[$dataOff + 3]
            $h = ([int]$pngBytes[$dataOff + 4] -shl 24) -bor ([int]$pngBytes[$dataOff + 5] -shl 16) -bor ([int]$pngBytes[$dataOff + 6] -shl 8) -bor [int]$pngBytes[$dataOff + 7]
            $bitDepth = [int]$pngBytes[$dataOff + 8]
            $colorType = [int]$pngBytes[$dataOff + 9]
        } elseif ($type -eq 'IDAT') {
            $idat.Write($pngBytes, $dataOff, $len)
        } elseif ($type -eq 'IEND') {
            break
        }
        $pos += 12 + $len
    }
    if ($w -le 0 -or $h -le 0 -or $bitDepth -ne 8 -or ($colorType -ne 6 -and $colorType -ne 2)) {
        $idat.Dispose()
        return $null
    }

    $zBytes = $idat.ToArray()
    $zStream = New-Object IO.MemoryStream ($zBytes, 2, ($zBytes.Length - 2))
    $deflate = New-Object IO.Compression.DeflateStream ($zStream, [IO.Compression.CompressionMode]::Decompress)
    $rawStream = New-Object IO.MemoryStream
    $deflate.CopyTo($rawStream)
    $deflate.Dispose(); $zStream.Dispose(); $idat.Dispose()
    $raw = $rawStream.ToArray()
    $rawStream.Dispose()

    $bpp = if ($colorType -eq 6) { 4 } else { 3 }
    $stride = $w * $bpp
    $pixels = New-Object byte[] ($h * $stride)
    for ($y = 0; $y -lt $h; $y++) {
        $f = [int]$raw[$y * ($stride + 1)]
        $srcRow = $y * ($stride + 1) + 1
        $dstRow = $y * $stride
        for ($x = 0; $x -lt $stride; $x++) {
            $val = [int]$raw[$srcRow + $x]
            $a = if ($x -ge $bpp) { [int]$pixels[$dstRow + $x - $bpp] } else { 0 }
            $b = if ($y -gt 0) { [int]$pixels[$dstRow - $stride + $x] } else { 0 }
            $c = if ($x -ge $bpp -and $y -gt 0) { [int]$pixels[$dstRow - $stride + $x - $bpp] } else { 0 }
            $recon = switch ($f) {
                0 { $val }
                1 { ($val + $a) -band 0xFF }
                2 { ($val + $b) -band 0xFF }
                3 { ($val + (($a + $b) -shr 1)) -band 0xFF }
                4 {
                    $p = $a + $b - $c
                    $pa = [math]::Abs($p - $a); $pb = [math]::Abs($p - $b); $pc = [math]::Abs($p - $c)
                    $pr = if ($pa -le $pb -and $pa -le $pc) { $a } elseif ($pb -le $pc) { $b } else { $c }
                    ($val + $pr) -band 0xFF
                }
                default { $val }
            }
            $pixels[$dstRow + $x] = [byte]$recon
        }
    }

    if ($bpp -eq 4) {
        return @{ Width = $w; Height = $h; Rgba = $pixels }
    }
    $rgba = New-Object byte[] ($w * $h * 4)
    for ($i = 0; $i -lt ($w * $h); $i++) {
        $rgba[$i * 4] = $pixels[$i * 3]
        $rgba[$i * 4 + 1] = $pixels[$i * 3 + 1]
        $rgba[$i * 4 + 2] = $pixels[$i * 3 + 2]
        $rgba[$i * 4 + 3] = 255
    }
    return @{ Width = $w; Height = $h; Rgba = $rgba }
}

# Box-downsamples + alpha-over composites an RGBA PNG icon onto a top-down RGB24 canvas with optional drop shadow.
function Draw-ScaledIconOverRgb24(
    [byte[]] $dstRgb, [int] $dstW, [int] $dstH,
    [hashtable] $icon, [int] $dstX, [int] $dstY, [int] $targetSize, [bool] $withShadow
) {
    $srcW = [int]$icon.Width; $srcH = [int]$icon.Height; $src = [byte[]]$icon.Rgba
    $scale = $srcW / [double]$targetSize
    # Pre-sample icon RGBA using area averaging for razor-sharp downsampling.
    $sampled = New-Object byte[] ($targetSize * $targetSize * 4)
    for ($dy = 0; $dy -lt $targetSize; $dy++) {
        $sy0 = $dy * $scale; $sy1 = ($dy + 1) * $scale
        $iy0 = [int][math]::Floor($sy0); $iy1 = [int][math]::Min($srcH - 1, [int][math]::Floor($sy1))
        for ($dx = 0; $dx -lt $targetSize; $dx++) {
            $sx0 = $dx * $scale; $sx1 = ($dx + 1) * $scale
            $ix0 = [int][math]::Floor($sx0); $ix1 = [int][math]::Min($srcW - 1, [int][math]::Floor($sx1))
            $sumR = 0.0; $sumG = 0.0; $sumB = 0.0; $sumA = 0.0; $wTot = 0.0
            for ($sy = $iy0; $sy -le $iy1; $sy++) {
                $wy = [math]::Min($sy + 1.0, $sy1) - [math]::Max([double]$sy, $sy0)
                if ($wy -le 0.0) { continue }
                for ($sx = $ix0; $sx -le $ix1; $sx++) {
                    $wx = [math]::Min($sx + 1.0, $sx1) - [math]::Max([double]$sx, $sx0)
                    if ($wx -le 0.0) { continue }
                    $wt = $wx * $wy
                    $p = ($sy * $srcW + $sx) * 4
                    $a = $src[$p + 3] / 255.0
                    # Premultiply so transparent edges never bleed dark fringes.
                    $sumR += $src[$p] * $a * $wt
                    $sumG += $src[$p + 1] * $a * $wt
                    $sumB += $src[$p + 2] * $a * $wt
                    $sumA += $a * $wt
                    $wTot += $wt
                }
            }
            $out = ($dy * $targetSize + $dx) * 4
            if ($wTot -gt 0.0 -and $sumA -gt 0.0) {
                $avgA = $sumA / $wTot
                $sampled[$out]     = [byte][math]::Min(255, [int][math]::Round($sumR / $sumA))
                $sampled[$out + 1] = [byte][math]::Min(255, [int][math]::Round($sumG / $sumA))
                $sampled[$out + 2] = [byte][math]::Min(255, [int][math]::Round($sumB / $sumA))
                $sampled[$out + 3] = [byte][math]::Min(255, [int][math]::Round($avgA * 255.0))
            }
        }
    }

    # Soft shadow pass (downward offset +3 px) using icon alpha mask.
    if ($withShadow) {
        $shOffX = 2; $shOffY = 4
        for ($dy = 0; $dy -lt $targetSize; $dy++) {
            $ty = $dstY + $dy + $shOffY
            if ($ty -lt 0 -or $ty -ge $dstH) { continue }
            for ($dx = 0; $dx -lt $targetSize; $dx++) {
                $tx = $dstX + $dx + $shOffX
                if ($tx -lt 0 -or $tx -ge $dstW) { continue }
                $a = ($sampled[($dy * $targetSize + $dx) * 4 + 3] / 255.0) * 0.38
                if ($a -le 0.005) { continue }
                $d = ($ty * $dstW + $tx) * 3
                $inv = 1.0 - $a
                $dstRgb[$d]     = [byte][int][math]::Round($dstRgb[$d] * $inv + 8 * $a)
                $dstRgb[$d + 1] = [byte][int][math]::Round($dstRgb[$d + 1] * $inv + 12 * $a)
                $dstRgb[$d + 2] = [byte][int][math]::Round($dstRgb[$d + 2] * $inv + 28 * $a)
            }
        }
    }

    # Foreground alpha-over composite.
    for ($dy = 0; $dy -lt $targetSize; $dy++) {
        $ty = $dstY + $dy
        if ($ty -lt 0 -or $ty -ge $dstH) { continue }
        for ($dx = 0; $dx -lt $targetSize; $dx++) {
            $tx = $dstX + $dx
            if ($tx -lt 0 -or $tx -ge $dstW) { continue }
            $s = ($dy * $targetSize + $dx) * 4
            $a = $sampled[$s + 3] / 255.0
            if ($a -le 0.002) { continue }
            $d = ($ty * $dstW + $tx) * 3
            $inv = 1.0 - $a
            $dstRgb[$d]     = [byte][math]::Min(255, [int][math]::Round($sampled[$s] * $a + $dstRgb[$d] * $inv))
            $dstRgb[$d + 1] = [byte][math]::Min(255, [int][math]::Round($sampled[$s + 1] * $a + $dstRgb[$d + 1] * $inv))
            $dstRgb[$d + 2] = [byte][math]::Min(255, [int][math]::Round($sampled[$s + 2] * $a + $dstRgb[$d + 2] * $inv))
        }
    }
}

# Branded Welcome/Exit left sidebar (164x312): Classic Indigo gradient, stationery rules, brass divider, and large 124x124 Nib & Leaf emblem.
function New-BannerSideBmp([string] $path, [hashtable] $icon256) {
    $w = 164; $h = 312
    $rgb = New-Object byte[] ($w * $h * 3)
    for ($y = 0; $y -lt $h; $y++) {
        $t = $y / [double]($h - 1)
        $bgR = [int][math]::Round(22 + (39 - 22) * $t)
        $bgG = [int][math]::Round(35 + (61 - 35) * $t)
        $bgB = [int][math]::Round(82 + (135 - 82) * $t)
        $isRule = ($y -ge 36 -and $y -le 276 -and (($y - 36) % 24) -eq 0)
        for ($x = 0; $x -lt $w; $x++) {
            $d = ($y * $w + $x) * 3
            if ($x -eq ($w - 1)) {
                $rgb[$d] = 218; $rgb[$d + 1] = 222; $rgb[$d + 2] = 232
            } elseif ($x -ge ($w - 4) -and $x -le ($w - 2)) {
                if ($x -eq ($w - 3)) {
                    $rgb[$d] = 224; $rgb[$d + 1] = 190; $rgb[$d + 2] = 118
                } else {
                    $rgb[$d] = 198; $rgb[$d + 1] = 155; $rgb[$d + 2] = 75
                }
            } elseif ($isRule -and $x -ge 16 -and $x -le ($w - 20)) {
                $rgb[$d] = 48; $rgb[$d + 1] = 70; $rgb[$d + 2] = 146
            } else {
                $rgb[$d] = [byte]$bgR; $rgb[$d + 1] = [byte]$bgG; $rgb[$d + 2] = [byte]$bgB
            }
        }
    }
    if ($null -ne $icon256) {
        Draw-ScaledIconOverRgb24 $rgb $w $h $icon256 19 72 124 $true
    }
    Write-Rgb24Bmp $path $w $h $rgb
}

# Branded Interior top banner (493x58): warm ivory header + Classic Indigo left bar + satin brass hairline + 44x44 Nib & Leaf emblem.
function New-TopBannerBmp([string] $path, [hashtable] $icon256) {
    $w = 493; $h = 58
    $rgb = New-Object byte[] ($w * $h * 3)
    for ($y = 0; $y -lt $h; $y++) {
        for ($x = 0; $x -lt $w; $x++) {
            $d = ($y * $w + $x) * 3
            if ($x -lt 6) {
                $rgb[$d] = 31; $rgb[$d + 1] = 48; $rgb[$d + 2] = 110
            } elseif ($y -ge ($h - 2)) {
                $rgb[$d] = 198; $rgb[$d + 1] = 155; $rgb[$d + 2] = 75
            } elseif ($x -ge ($w - 6)) {
                $rgb[$d] = 236; $rgb[$d + 1] = 239; $rgb[$d + 2] = 248
            } else {
                $rgb[$d] = 252; $rgb[$d + 1] = 251; $rgb[$d + 2] = 248
            }
        }
    }
    if ($null -ne $icon256) {
        Draw-ScaledIconOverRgb24 $rgb $w $h $icon256 436 7 44 $false
    }
    Write-Rgb24Bmp $path $w $h $rgb
}

# Writes a 16x16 32-bit BGRA ICO from a 16-row ASCII art grid + color map (char -> @(R,G,B,A)).
function Write-Icon16([string] $path, [string[]] $grid, [hashtable] $colors) {
    $w = 16; $h = 16
    $dibSize = 40 + ($w * $h * 4) + ($h * 4) # header + XOR BGRA + AND 1bpp (4-byte aligned rows)
    $buf = New-Object byte[] (6 + 16 + $dibSize)

    # ICONDIR
    Write-UInt16LE $buf 0 0
    Write-UInt16LE $buf 2 1 # type = ICO
    Write-UInt16LE $buf 4 1 # count = 1

    # ICONDIRENTRY
    $buf[6] = 16; $buf[7] = 16; $buf[8] = 0; $buf[9] = 0
    Write-UInt16LE $buf 10 1   # planes
    Write-UInt16LE $buf 12 32  # bpp
    Write-Int32LE $buf 14 $dibSize
    Write-Int32LE $buf 18 22   # offset to DIB

    # BITMAPINFOHEADER
    $dibOff = 22
    Write-Int32LE $buf $dibOff 40
    Write-Int32LE $buf ($dibOff + 4) $w
    Write-Int32LE $buf ($dibOff + 8) ($h * 2) # XOR + AND height
    Write-UInt16LE $buf ($dibOff + 12) 1
    Write-UInt16LE $buf ($dibOff + 14) 32
    Write-Int32LE $buf ($dibOff + 20) ($w * $h * 4)

    $xorOff = $dibOff + 40
    $andOff = $xorOff + ($w * $h * 4)
    for ($y = 0; $y -lt $h; $y++) {
        $rowStr = $grid[$h - 1 - $y]
        $maskBits = 0
        for ($x = 0; $x -lt $w; $x++) {
            $ch = [string]$rowStr[$x]
            $rgba = $colors[$ch]
            $pOff = $xorOff + (($y * $w + $x) * 4)
            $buf[$pOff] = [byte]$rgba[2]     # B
            $buf[$pOff + 1] = [byte]$rgba[1] # G
            $buf[$pOff + 2] = [byte]$rgba[0] # R
            $buf[$pOff + 3] = [byte]$rgba[3] # A
            if ($rgba[3] -eq 0) {
                $maskBits = $maskBits -bor (1 -shl (15 - $x))
            }
        }
        $mOff = $andOff + ($y * 4)
        $buf[$mOff] = [byte](($maskBits -shr 8) -band 0xFF)
        $buf[$mOff + 1] = [byte]($maskBits -band 0xFF)
    }

    [IO.File]::WriteAllBytes($path, $buf)
}

function New-BrowseIcons([string] $upPath, [string] $newFolderPath) {
    $palette = @{
        '.' = @(0, 0, 0, 0)
        'o' = @(122, 92, 34, 255)    # warm bronze folder outline
        'f' = @(238, 198, 108, 255)  # warm folder fill
        'h' = @(252, 232, 168, 255)  # folder highlight
        'a' = @(31, 48, 110, 255)    # Classic Indigo arrow
        'g' = @(34, 134, 72, 255)    # emerald plus for New Folder
        'w' = @(255, 255, 255, 255)  # white contrast halo
    }
    $upGrid = @(
        '................'
        '................'
        '..ooooo.........'
        '.ohhhhho........'
        '.ohhhhhhhhhhho..'
        '.offfffaaffffo..'
        '.offffaaaafffo..'
        '.offfaaaaaaffo..'
        '.offfaaaaafffo..'
        '.offfffaaffffo..'
        '.offfffaaffffo..'
        '.offfffaaaaffo..'
        '.offfffffffaffo.'
        '..ooooooooooo...'
        '................'
        '................'
    )
    $newGrid = @(
        '................'
        '..........gg....'
        '..ooooo...gg....'
        '.ohhhhho.gggg...'
        '.ohhhhhhhgggg...'
        '.offffffffgg.o..'
        '.offffffffggfo..'
        '.offfffffffffo..'
        '.offfffffffffo..'
        '.offfffffffffo..'
        '.offfffffffffo..'
        '.offfffffffffo..'
        '.offfffffffffo..'
        '..ooooooooooo...'
        '................'
        '................'
    )
    Write-Icon16 $upPath $upGrid $palette
    Write-Icon16 $newFolderPath $newGrid $palette
}

# Extracts the 32x32 PNG-encoded entry from an ICO (such as DD28's ecclesiapdf.ico) and writes a standard
# uncompressed 32x32 32-bit BGRA DIB ICO for Windows Installer's UI Icon control (which rejects PNG-compressed
# ICO entries with DEBUG: Error 2863). Reuses Read-PngRgba (no duplicated PNG decoder).
function Convert-PngIcoToDib32Ico([string] $srcIcoPath, [string] $dstIcoPath) {
    $src = [IO.File]::ReadAllBytes($srcIcoPath)
    $count = [BitConverter]::ToUInt16($src, 4)
    $pngBytes = $null
    for ($i = 0; $i -lt $count; $i++) {
        $off = 6 + ($i * 16)
        $ew = if ($src[$off] -eq 0) { 256 } else { [int]$src[$off] }
        $eh = if ($src[$off + 1] -eq 0) { 256 } else { [int]$src[$off + 1] }
        if ($ew -eq 32 -and $eh -eq 32) {
            $sz = [BitConverter]::ToInt32($src, $off + 8)
            $imgOff = [BitConverter]::ToInt32($src, $off + 12)
            $pngBytes = New-Object byte[] $sz
            [Array]::Copy($src, $imgOff, $pngBytes, 0, $sz)
            break
        }
    }
    $decoded = Read-PngRgba $pngBytes
    if ($null -eq $decoded -or $decoded.Width -ne 32 -or $decoded.Height -ne 32) {
        Copy-Item -Force $srcIcoPath $dstIcoPath
        return
    }

    $w = 32; $h = 32; $rgba = [byte[]]$decoded.Rgba; $stride = $w * 4
    # Write 32x32 uncompressed 32-bit BGRA DIB ICO (40-byte header + 4096 XOR + 128 AND).
    $dibSize = 40 + ($w * $h * 4) + ($h * 4)
    $buf = New-Object byte[] (6 + 16 + $dibSize)
    Write-UInt16LE $buf 0 0; Write-UInt16LE $buf 2 1; Write-UInt16LE $buf 4 1
    $buf[6] = 32; $buf[7] = 32; $buf[8] = 0; $buf[9] = 0
    Write-UInt16LE $buf 10 1; Write-UInt16LE $buf 12 32
    Write-Int32LE $buf 14 $dibSize; Write-Int32LE $buf 18 22
    $dibOff = 22
    Write-Int32LE $buf $dibOff 40; Write-Int32LE $buf ($dibOff + 4) $w; Write-Int32LE $buf ($dibOff + 8) ($h * 2)
    Write-UInt16LE $buf ($dibOff + 12) 1; Write-UInt16LE $buf ($dibOff + 14) 32
    Write-Int32LE $buf ($dibOff + 20) ($w * $h * 4)

    $xorOff = $dibOff + 40
    $andOff = $xorOff + ($w * $h * 4)
    for ($y = 0; $y -lt $h; $y++) {
        $srcRow = ($h - 1 - $y) * $stride
        [uint32]$maskBits = 0
        for ($x = 0; $x -lt $w; $x++) {
            $s = $srcRow + ($x * 4)
            $d = $xorOff + (($y * $w + $x) * 4)
            $buf[$d] = $rgba[$s + 2]; $buf[$d + 1] = $rgba[$s + 1]; $buf[$d + 2] = $rgba[$s]; $buf[$d + 3] = $rgba[$s + 3]
            if ($rgba[$s + 3] -lt 128) { $maskBits = $maskBits -bor ([uint32]1 -shl (31 - $x)) }
        }
        $mOff = $andOff + ($y * 4)
        $buf[$mOff]     = [byte](($maskBits -shr 24) -band 0xFF)
        $buf[$mOff + 1] = [byte](($maskBits -shr 16) -band 0xFF)
        $buf[$mOff + 2] = [byte](($maskBits -shr 8) -band 0xFF)
        $buf[$mOff + 3] = [byte]($maskBits -band 0xFF)
    }
    [IO.File]::WriteAllBytes($dstIcoPath, $buf)
}

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
    $name = "EcclesiaPDF-v$Date.$Release-v$version-windows-x64"
    $msi = Join-Path $OutDir "$name.msi"
}
finally { Pop-Location }

# The installed files: key (the name inside the cabinet), MSI file name (short|long), source.
$files = @(
    @{ Key = 'ecclesiapdf.exe'; Name = 'ECCLES~1.EXE|ecclesiapdf.exe'; Path = $Exe }
    @{ Key = 'msvcp140.dll'; Name = 'msvcp140.dll'; Path = Join-Path $CrtDir 'msvcp140.dll' }
    @{ Key = 'vcruntime140.dll'; Name = 'VCRUNT~1.DLL|vcruntime140.dll'; Path = Join-Path $CrtDir 'vcruntime140.dll' }
    @{ Key = 'vcruntime140_1.dll'; Name = 'VCRUNT~2.DLL|vcruntime140_1.dll'; Path = Join-Path $CrtDir 'vcruntime140_1.dll' }
)

# The cabinet, LZX:21 as plan §13.1's size baseline measures it.
$work = Join-Path ([IO.Path]::GetTempPath()) "ecclesiapdf-msi-$PID"
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

    # Generate branded UI bitmap and browse-toolbar icon assets into the temp directory.
    $bannerSidePath = Join-Path $work 'BannerSide.bmp'
    $topBannerPath = Join-Path $work 'TopBanner.bmp'
    $upIcoPath = Join-Path $work 'UpIco.ico'
    $newFolderIcoPath = Join-Path $work 'NewFolderIco.ico'
    $appIconPath = Join-Path $root 'packaging/windows/ecclesiapdf.ico'
    $src256Path = Join-Path $root 'design/icon/png/ecclesiapdf-256.png'
    $icon256 = if (Test-Path $src256Path) { Read-PngRgba ([IO.File]::ReadAllBytes($src256Path)) } else { $null }
    $appIcon32Path = Join-Path $work 'AppIcon32.ico'
    New-BannerSideBmp $bannerSidePath $icon256
    New-TopBannerBmp $topBannerPath $icon256
    New-BrowseIcons $upIcoPath $newFolderIcoPath
    Convert-PngIcoToDib32Ico $appIconPath $appIcon32Path

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
        Icon                   = '`Name` CHAR(72) NOT NULL, `Data` OBJECT NOT NULL PRIMARY KEY `Name`'
        Binary                 = '`Name` CHAR(72) NOT NULL, `Data` OBJECT NOT NULL PRIMARY KEY `Name`'
        TextStyle              = '`TextStyle` CHAR(72) NOT NULL, `FaceName` CHAR(32) NOT NULL, `Size` SHORT NOT NULL, `Color` LONG, `StyleBits` SHORT PRIMARY KEY `TextStyle`'
        Dialog                 = '`Dialog` CHAR(72) NOT NULL, `HCentering` SHORT NOT NULL, `VCentering` SHORT NOT NULL, `Width` SHORT NOT NULL, `Height` SHORT NOT NULL, `Attributes` LONG, `Title` CHAR(128) LOCALIZABLE, `Control_First` CHAR(50) NOT NULL, `Control_Default` CHAR(50), `Control_Cancel` CHAR(50) PRIMARY KEY `Dialog`'
        Control                = '`Dialog_` CHAR(72) NOT NULL, `Control` CHAR(50) NOT NULL, `Type` CHAR(20) NOT NULL, `X` SHORT NOT NULL, `Y` SHORT NOT NULL, `Width` SHORT NOT NULL, `Height` SHORT NOT NULL, `Attributes` LONG, `Property` CHAR(50), `Text` LONGCHAR LOCALIZABLE, `Control_Next` CHAR(50), `Help` CHAR(50) LOCALIZABLE PRIMARY KEY `Dialog_`, `Control`'
        ControlEvent           = '`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Event` CHAR(50) NOT NULL, `Argument` CHAR(255) NOT NULL, `Condition` CHAR(255) NOT NULL, `Ordering` SHORT PRIMARY KEY `Dialog_`, `Control_`, `Event`, `Argument`, `Condition`'
        ControlCondition       = '`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Action` CHAR(50) NOT NULL, `Condition` CHAR(255) NOT NULL PRIMARY KEY `Dialog_`, `Control_`, `Action`, `Condition`'
        RadioButton            = '`Property` CHAR(72) NOT NULL, `Order` SHORT NOT NULL, `Value` CHAR(64) NOT NULL, `X` SHORT NOT NULL, `Y` SHORT NOT NULL, `Width` SHORT NOT NULL, `Height` SHORT NOT NULL, `Text` CHAR(255) LOCALIZABLE, `Help` CHAR(50) LOCALIZABLE PRIMARY KEY `Property`, `Order`'
        CheckBox               = '`Property` CHAR(72) NOT NULL, `Value` CHAR(64) PRIMARY KEY `Property`'
        EventMapping           = '`Dialog_` CHAR(72) NOT NULL, `Control_` CHAR(50) NOT NULL, `Event` CHAR(50) NOT NULL, `Attribute` CHAR(50) NOT NULL PRIMARY KEY `Dialog_`, `Control_`, `Event`'
        UIText                 = '`Key` CHAR(72) NOT NULL, `Text` CHAR(255) LOCALIZABLE PRIMARY KEY `Key`'
        ActionText             = '`Action` CHAR(72) NOT NULL, `Description` CHAR(255) LOCALIZABLE, `Template` CHAR(255) LOCALIZABLE PRIMARY KEY `Action`'
        Error                  = '`Error` SHORT NOT NULL, `Message` LONGCHAR LOCALIZABLE PRIMARY KEY `Error`'
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

    function Add-Stream([string] $table, [string] $streamName, [string] $filePath) {
        $view = Invoke-Com $db 'OpenView' @("SELECT ``Name``, ``Data`` FROM ``$table``")
        $record = Invoke-Com $installer 'CreateRecord' @(2)
        Set-ComProperty $record 'StringData' @(1, $streamName)
        [void] (Invoke-Com $record 'SetStream' @(2, $filePath))
        [void] (Invoke-Com $view 'Modify' @(1, $record)) # msiViewModifyInsert
        [void] (Invoke-Com $view 'Close')
    }

    # --- Properties (MSI 5.0 Single Package Authoring defaults) ---
    $productCode = "{$([guid]::NewGuid().ToString().ToUpper())}"
    $properties = [ordered] @{
        ProductCode            = $productCode
        ProductName            = 'Ecclesia PDF'
        ProductVersion         = $version
        ProductLanguage        = '1033'
        Manufacturer           = 'Ecclesia PDF'
        UpgradeCode            = $UpgradeCode
        # Single Package Authoring: dual-purpose per-user (default, no UAC) or per-machine (UAC elevation).
        ALLUSERS               = '2'
        MSIINSTALLPERUSER      = '1'
        INSTALLSCOPE           = 'PerUser'
        SCOPESUMMARY           = 'Current user only (no administrator privileges required)'
        REGISTERFILETYPES      = '1'
        FILEASSOCSUMMARY       = 'Register for supported file types (.pdf, .epub, .xps, ...)'
        OPENDEFAULTAPPS        = '1'
        LAUNCHAPP              = '1'
        _BrowseProperty        = 'INSTALLDIR'
        ARPNOMODIFY            = '1'
        ARPPRODUCTICON         = 'AppIcon.ico'
        DefaultUIFont          = 'DlgFont8'
        ErrorDialog            = 'ErrorDialog'
        # Forward user-selected scope, destination directory, and file association choices across the UAC elevation boundary.
        SecureCustomProperties = 'OLDERFOUND;NEWERFOUND;INSTALLDIR;ALLUSERS;MSIINSTALLPERUSER;INSTALLSCOPE;LAUNCHAPP;REGISTERFILETYPES;OPENDEFAULTAPPS;MACHINEINSTALLDIR;USERINSTALLDIR;USERCUSTOMDIR;MAINTMODE'
    }
    foreach ($key in $properties.Keys) { Add-Row Property @($key, $properties[$key]) }

    # --- Directories ---
    Add-Row Directory @('TARGETDIR', $null, 'SourceDir')
    Add-Row Directory @('LocalAppDataFolder', 'TARGETDIR', '.')
    Add-Row Directory @('UserPrograms', 'LocalAppDataFolder', 'Programs')
    Add-Row Directory @('INSTALLDIR', 'UserPrograms', 'ECCLES~1|EcclesiaPDF')
    Add-Row Directory @('ProgramFiles64Folder', 'TARGETDIR', '.')
    Add-Row Directory @('ProgramMenuFolder', 'TARGETDIR', '.')
    Add-Row Directory @('System64Folder', 'TARGETDIR', '.')

    # --- Components, Features, Files, Media ---
    # 4: registry KeyPath (switches HKCU/HKLM automatically via Root = -1); 256: 64-bit component.
    Add-Row Component @('App', $AppComponentId, 'INSTALLDIR', 260, $null, 'AppInstalled')
    Add-Row Component @('StartMenu', $ShortcutComponentId, 'ProgramMenuFolder', 260, $null, 'ShortcutInstalled')
    Add-Row Component @('FileAssoc', $FileAssocComponentId, 'INSTALLDIR', 260, 'REGISTERFILETYPES = "1"', 'FileAssocRegistered')
    Add-Row Feature @('Main', $null, 'Ecclesia PDF', 'The Ecclesia PDF reader, annotator and notebook', 1, 1, 'INSTALLDIR', 0)
    Add-Row FeatureComponents @('Main', 'App')
    Add-Row FeatureComponents @('Main', 'StartMenu')
    Add-Row FeatureComponents @('Main', 'FileAssoc')

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

    # --- Registry (Root = -1: HKCU for per-user, HKLM for per-machine), RemoveFile, Shortcut ---
    $regKey = 'Software\EcclesiaPDF\Installer'
    Add-Row Registry @('AppInstalled', -1, $regKey, 'InstallDir', '[INSTALLDIR]', 'App')
    Add-Row Registry @('ShortcutInstalled', -1, $regKey, 'StartMenuShortcut', '#1', 'StartMenu')
    Add-Row Registry @('InstallerKey', -1, $regKey, '-', $null, 'App')

    # Dual-scope Default Programs / file type associations (Root = -1: HKCU for per-user, HKLM for per-machine).
    Add-Row Registry @('FileAssocRegistered', -1, $regKey, 'FileAssociations', '#1', 'FileAssoc')
    Add-Row Registry @('ProgIdDesc', -1, 'Software\Classes\EcclesiaPDF.Document', $null, 'Ecclesia PDF Document', 'FileAssoc')
    Add-Row Registry @('ProgIdIcon', -1, 'Software\Classes\EcclesiaPDF.Document\DefaultIcon', $null, '[INSTALLDIR]ecclesiapdf.exe,0', 'FileAssoc')
    Add-Row Registry @('ProgIdOpenCmd', -1, 'Software\Classes\EcclesiaPDF.Document\shell\open\command', $null, '"[INSTALLDIR]ecclesiapdf.exe" "%1"', 'FileAssoc')
    Add-Row Registry @('CapAppName', -1, 'Software\EcclesiaPDF\Capabilities', 'ApplicationName', 'Ecclesia PDF', 'FileAssoc')
    Add-Row Registry @('CapAppDesc', -1, 'Software\EcclesiaPDF\Capabilities', 'ApplicationDescription', 'Fast PDF, e-book, and annotation reader', 'FileAssoc')
    Add-Row Registry @('RegAppCap', -1, 'Software\RegisteredApplications', 'Ecclesia PDF', 'Software\EcclesiaPDF\Capabilities', 'FileAssoc')
    $fileExtensions = @('.pdf', '.xps', '.oxps', '.epub', '.cbz', '.cbr', '.mobi', '.fb2', '.djvu', '.expdf')
    foreach ($ext in $fileExtensions) {
        $extTag = $ext.TrimStart('.')
        Add-Row Registry @("OpenWith_$extTag", -1, "Software\Classes\$ext\OpenWithProgids", 'EcclesiaPDF.Document', '', 'FileAssoc')
        Add-Row Registry @("CapExt_$extTag", -1, 'Software\EcclesiaPDF\Capabilities\FileAssociations', $ext, 'EcclesiaPDF.Document', 'FileAssoc')
    }

    Add-Row RemoveFile @('RemoveInstallDir', 'App', $null, 'INSTALLDIR', 2)
    Add-Row Shortcut @('StartMenu', 'ProgramMenuFolder', 'ECCLES~1|Ecclesia PDF', 'StartMenu', '[#ecclesiapdf.exe]',
        $null, 'Read, annotate and organise PDFs', $null, $null, $null, $null, 'INSTALLDIR')

    # --- Upgrades and Launch Conditions ---
    # Same or older builds are replaced (512: up to this version inclusive); a newer one stops the install (2: detect only).
    Add-Row Upgrade @($UpgradeCode, $null, $version, $null, 512, $null, 'OLDERFOUND')
    Add-Row Upgrade @($UpgradeCode, $version, $null, $null, 2, $null, 'NEWERFOUND')
    Add-Row LaunchCondition @('NOT NEWERFOUND', 'A newer version of Ecclesia PDF is already installed.')

    # --- Custom Actions ---
    # Type 51 = set property with formatted text; Type 35 = set directory with formatted text (after CostFinalize).
    # Note: [%ProgramW6432] always holds the true 64-bit Program Files path even when MSIINSTALLPERUSER=1 redirects
    # ProgramFiles64Folder to %LOCALAPPDATA%\Programs in Windows Installer 5.0.
    Add-Row CustomAction @('MarkCustomDir', 51, 'USERCUSTOMDIR', '1')
    Add-Row CustomAction @('InitUserDir', 51, 'USERINSTALLDIR', '[LocalAppDataFolder]Programs\EcclesiaPDF\')
    Add-Row CustomAction @('InitMachineDir', 51, 'MACHINEINSTALLDIR', '[%ProgramW6432]\EcclesiaPDF\')
    Add-Row CustomAction @('InitMachineDirFallback', 51, 'MACHINEINSTALLDIR', '[ProgramFiles64Folder]EcclesiaPDF\')
    Add-Row CustomAction @('SyncScopeMachine', 51, 'INSTALLSCOPE', 'PerMachine')
    Add-Row CustomAction @('SetScopeSummaryUser', 51, 'SCOPESUMMARY', 'Current user only (no administrator privileges required)')
    Add-Row CustomAction @('SetScopeSummaryMachine', 51, 'SCOPESUMMARY', 'All users on this computer (requires administrator privileges)')
    Add-Row CustomAction @('SetInstallDirMachine', 35, 'INSTALLDIR', '[MACHINEINSTALLDIR]')
    # Type 98 = 34 (EXE in Directory) + 64 (synchronous, ignore exit code): runs invisibly via conhost --headless
    # before InstallValidate so any running ecclesiapdf.exe saves its session via WM_CLOSE and exits cleanly
    # (falling back to /F if stuck), preventing locked-file reboot prompts on install, upgrade, repair, or remove.
    $stopRunningCmd = '"[System64Folder]conhost.exe" --headless "[System64Folder]cmd.exe" /d /c "tasklist /NH | findstr /I /B ecclesiapdf.exe >nul && (taskkill /IM ecclesiapdf.exe /T >nul 2>&1 & ping -n 2 127.0.0.1 >nul & taskkill /F /IM ecclesiapdf.exe /T >nul 2>&1)"'
    Add-Row CustomAction @('StopRunningApp', 98, 'System64Folder', $stopRunningCmd)
    # Type 226 = 34 (EXE in Directory) + 192 (async, ignore exit code); invoked from ExitDialog (unelevated user session).
    Add-Row CustomAction @('LaunchApp', 226, 'INSTALLDIR', '"[INSTALLDIR]ecclesiapdf.exe"')
    Add-Row CustomAction @('OpenDefaultApps', 226, 'System64Folder', '"[System64Folder]cmd.exe" /c start "" "ms-settings:defaultapps"')

    # --- Text Styles ---
    # Color is BGR (65536*B + 256*G + R); $null uses the Windows system text color (High Contrast friendly).
    $indigoBgr = (65536 * 110) + (256 * 48) + 31 # Classic Indigo (#1F306E)
    Add-Row TextStyle @('DlgFont8', 'Segoe UI', 9, $null, 0)
    Add-Row TextStyle @('DlgFontBold8', 'Segoe UI', 9, $null, 1)
    Add-Row TextStyle @('DlgTitleFont', 'Segoe UI', 13, $indigoBgr, 1)
    Add-Row TextStyle @('BannerTitleFont', 'Segoe UI', 10, $indigoBgr, 1)

    # --- Control Attribute Constants ---
    $ATTR_VISIBLE       = 1
    $ATTR_ENABLED       = 2
    $ATTR_SUNKEN        = 4
    $ATTR_INDIRECT      = 8
    $ATTR_TRANSPARENT   = 65536
    $ATTR_NOPREFIX      = 131072
    $ATTR_PROGRESS95    = 65536
    $ATTR_REMOVABLE_VOL = 65536
    $ATTR_FIXED_VOL     = 131072
    $ATTR_REMOTE_VOL    = 262144
    $ATTR_ICON          = 524288
    $ATTR_FIXED_SIZE    = 1048576
    $ATTR_ICON_16       = 2097152
    $ATTR_ICON_32       = 4194304
    $ATTR_ELEVATION     = 8388608

    $CTL_BTN            = $ATTR_VISIBLE -bor $ATTR_ENABLED
    $CTL_BTN_DISABLED   = $ATTR_VISIBLE
    $CTL_BTN_ELEVATED   = $CTL_BTN -bor $ATTR_ELEVATION
    $CTL_BTN_ICON16     = $CTL_BTN -bor $ATTR_ICON -bor $ATTR_FIXED_SIZE -bor $ATTR_ICON_16
    $CTL_ICON_32        = $ATTR_VISIBLE -bor $ATTR_ICON -bor $ATTR_FIXED_SIZE -bor $ATTR_ICON_32
    $CTL_TEXT           = $ATTR_VISIBLE -bor $ATTR_ENABLED
    $CTL_TEXT_BANNER    = $CTL_TEXT -bor $ATTR_TRANSPARENT
    $CTL_TEXT_PATH      = $CTL_TEXT -bor $ATTR_NOPREFIX
    $CTL_PATHEDIT       = $CTL_BTN -bor $ATTR_SUNKEN
    $CTL_PATHEDIT_IND   = $CTL_PATHEDIT -bor $ATTR_INDIRECT
    $CTL_DIRCOMBO       = $CTL_BTN -bor $ATTR_INDIRECT -bor $ATTR_REMOVABLE_VOL -bor $ATTR_FIXED_VOL -bor $ATTR_REMOTE_VOL
    $CTL_DIRLIST        = $CTL_BTN -bor $ATTR_SUNKEN -bor $ATTR_INDIRECT
    $CTL_PROGRESS       = $ATTR_VISIBLE -bor $ATTR_PROGRESS95

    $DLG_MODAL          = 3      # Visible | Modal
    $DLG_WIZARD         = 7      # Visible | Modal | Minimize
    $DLG_MODELESS       = 5      # Visible | Minimize (for PrepareDialog and ProgressDialog)
    $DLG_ERROR          = 65539  # Visible | Modal | Error

    $dlgTitle = '[ProductName] Setup'

    # Helper to add the shared interior banner + bottom separator on wizard interior dialogs.
    function Add-InteriorChrome([string] $dialog, [string] $titleText, [string] $subtitleText) {
        Add-Row Control @($dialog, 'BannerBitmap', 'Bitmap', 0, 0, 370, 44, $ATTR_VISIBLE, $null, 'TopBanner', $null, $null)
        Add-Row Control @($dialog, 'BannerLine', 'Line', 0, 44, 370, 0, $ATTR_VISIBLE, $null, $null, $null, $null)
        Add-Row Control @($dialog, 'Title', 'Text', 15, 6, 305, 15, $CTL_TEXT_BANNER, $null, "{\BannerTitleFont}$titleText", $null, $null)
        Add-Row Control @($dialog, 'Description', 'Text', 15, 23, 305, 18, $CTL_TEXT_BANNER, $null, $subtitleText, $null, $null)
        Add-Row Control @($dialog, 'BottomLine', 'Line', 0, 234, 370, 0, $ATTR_VISIBLE, $null, $null, $null, $null)
    }

    # Helper to add the shared branded sidebar (with large Nib & Leaf emblem) on exterior dialogs.
    function Add-ExteriorChrome([string] $dialog) {
        Add-Row Control @($dialog, 'SideBitmap', 'Bitmap', 0, 0, 110, 234, $ATTR_VISIBLE, $null, 'BannerSide', $null, $null)
        Add-Row Control @($dialog, 'BottomLine', 'Line', 0, 234, 370, 0, $ATTR_VISIBLE, $null, $null, $null, $null)
    }

    # =========================================================================
    # 1. PrepareDialog (modeless, shown briefly while costing runs)
    # =========================================================================
    Add-Row Dialog @('PrepareDialog', 50, 50, 370, 270, $DLG_MODELESS, $dlgTitle, 'Cancel', 'Cancel', 'Cancel')
    Add-InteriorChrome 'PrepareDialog' 'Preparing to Install' 'Please wait while Windows prepares Ecclesia PDF Setup.'
    Add-Row Control @('PrepareDialog', 'WaitText', 'Text', 22, 84, 326, 36, $CTL_TEXT, $null,
        'Windows Installer is computing space requirements and preparing the installation wizard...', $null, $null)
    Add-Row Control @('PrepareDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN_DISABLED, $null, '< &Back', $null, $null)
    Add-Row Control @('PrepareDialog', 'Next', 'PushButton', 236, 243, 56, 17, $CTL_BTN_DISABLED, $null, '&Next >', $null, $null)
    Add-Row Control @('PrepareDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'Cancel', $null)
    Add-Row ControlEvent @('PrepareDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 2. WelcomeDialog (fresh install / upgrade entry)
    # =========================================================================
    Add-Row Dialog @('WelcomeDialog', 50, 50, 370, 270, $DLG_WIZARD, $dlgTitle, 'Next', 'Next', 'Cancel')
    Add-ExteriorChrome 'WelcomeDialog'
    Add-Row Control @('WelcomeDialog', 'Title', 'Text', 124, 20, 232, 32, $CTL_TEXT, $null,
        '{\DlgTitleFont}Welcome to Ecclesia PDF Setup', $null, $null)
    Add-Row Control @('WelcomeDialog', 'Intro', 'Text', 124, 62, 232, 36, $CTL_TEXT, $null,
        'This wizard will install Ecclesia PDF [ProductVersion] on your computer.', $null, $null)
    Add-Row Control @('WelcomeDialog', 'Details', 'Text', 124, 102, 232, 54, $CTL_TEXT, $null,
        'You can choose to install just for your account (no administrator rights needed) or for all users on this computer, and pick any destination folder.', $null, $null)
    Add-Row Control @('WelcomeDialog', 'Prompt', 'Text', 124, 176, 232, 28, $CTL_TEXT, $null,
        'Click Next to continue, or Cancel to exit Setup.', $null, $null)
    Add-Row Control @('WelcomeDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN_DISABLED, $null, '< &Back', $null, $null)
    Add-Row Control @('WelcomeDialog', 'Next', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Next >', 'Cancel', $null)
    Add-Row Control @('WelcomeDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'Next', $null)
    Add-Row ControlEvent @('WelcomeDialog', 'Next', 'NewDialog', 'ScopeDialog', '1', 1)
    Add-Row ControlEvent @('WelcomeDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 3. ScopeDialog (Per-User vs Per-Machine selection)
    # =========================================================================
    Add-Row Dialog @('ScopeDialog', 50, 50, 370, 270, $DLG_WIZARD, $dlgTitle, 'ScopeGroup', 'Next', 'Cancel')
    Add-InteriorChrome 'ScopeDialog' 'Installation Scope' 'Choose who should be able to use EcclesiaPDF on this computer.'
    Add-Row Control @('ScopeDialog', 'ScopePrompt', 'Text', 22, 56, 326, 14, $CTL_TEXT, $null,
        'Select how you want to install Ecclesia PDF:', $null, $null)
    Add-Row Control @('ScopeDialog', 'ScopeGroup', 'RadioButtonGroup', 22, 74, 326, 114, $CTL_BTN, 'INSTALLSCOPE', $null, 'Back', $null)
    Add-Row RadioButton @('INSTALLSCOPE', 1, 'PerUser', 4, 4, 318, 46,
        '{\DlgFontBold8}Install just for me (Current User) - installs in your user profile. No administrator permission required.', $null)
    Add-Row RadioButton @('INSTALLSCOPE', 2, 'PerMachine', 4, 58, 318, 46,
        '{\DlgFontBold8}Install for all users on this computer (Administrator) - installs in Program Files. Requires administrator permission (UAC).', $null)
    Add-Row Control @('ScopeDialog', 'ScopeHint', 'Text', 22, 196, 326, 24, $CTL_TEXT, $null,
        'You will be able to review or change the installation folder on the next screen.', $null, $null)
    Add-Row Control @('ScopeDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN, $null, '< &Back', 'Next', $null)
    Add-Row Control @('ScopeDialog', 'Next', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Next >', 'Cancel', $null)
    Add-Row Control @('ScopeDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'ScopeGroup', $null)

    Add-Row ControlEvent @('ScopeDialog', 'Back', 'NewDialog', 'WelcomeDialog', '1', 1)
    # Apply Per-User scope properties and update default INSTALLDIR when not customized.
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[ALLUSERS]', '2', 'INSTALLSCOPE = "PerUser"', 1)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[MSIINSTALLPERUSER]', '1', 'INSTALLSCOPE = "PerUser"', 2)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[SCOPESUMMARY]', 'Current user only (no administrator privileges required)', 'INSTALLSCOPE = "PerUser"', 3)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[INSTALLDIR]', '[USERINSTALLDIR]', 'INSTALLSCOPE = "PerUser" AND (NOT USERCUSTOMDIR OR INSTALLDIR = MACHINEINSTALLDIR)', 4)
    Add-Row ControlEvent @('ScopeDialog', 'Next', 'SetTargetPath', 'INSTALLDIR', 'INSTALLSCOPE = "PerUser"', 5)
    # Apply Per-Machine scope properties (keeps ALLUSERS=2, clears MSIINSTALLPERUSER so UAC elevates cleanly).
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[MSIINSTALLPERUSER]', '{}', 'INSTALLSCOPE = "PerMachine"', 6)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[ALLUSERS]', '2', 'INSTALLSCOPE = "PerMachine"', 7)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[SCOPESUMMARY]', 'All users on this computer (requires administrator privileges)', 'INSTALLSCOPE = "PerMachine"', 8)
    Add-Row ControlEvent @('ScopeDialog', 'Next', '[INSTALLDIR]', '[MACHINEINSTALLDIR]', 'INSTALLSCOPE = "PerMachine" AND (NOT USERCUSTOMDIR OR INSTALLDIR = USERINSTALLDIR)', 9)
    Add-Row ControlEvent @('ScopeDialog', 'Next', 'SetTargetPath', 'INSTALLDIR', 'INSTALLSCOPE = "PerMachine"', 10)
    Add-Row ControlEvent @('ScopeDialog', 'Next', 'NewDialog', 'FolderDialog', '1', 11)
    Add-Row ControlEvent @('ScopeDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 4. FolderDialog (Custom directory PathEdit + Browse... + File associations)
    # =========================================================================
    Add-Row Dialog @('FolderDialog', 50, 50, 370, 270, $DLG_WIZARD, $dlgTitle, 'FolderEdit', 'Next', 'Cancel')
    Add-InteriorChrome 'FolderDialog' 'Destination Folder' 'Click Next to install to the default folder, or choose any folder below.'
    Add-Row Control @('FolderDialog', 'FolderLabel', 'Text', 22, 62, 326, 14, $CTL_TEXT, $null,
        '&Install Ecclesia PDF to:', $null, $null)
    Add-Row Control @('FolderDialog', 'FolderEdit', 'PathEdit', 22, 78, 252, 18, $CTL_PATHEDIT, 'INSTALLDIR', $null, 'Browse', $null)
    Add-Row Control @('FolderDialog', 'Browse', 'PushButton', 282, 78, 66, 18, $CTL_BTN, $null, 'B&rowse...', 'FileAssocCheckBox', $null)
    Add-Row Control @('FolderDialog', 'ScopeLabel', 'Text', 22, 106, 326, 14, $CTL_TEXT, $null,
        '{\DlgFontBold8}Selected scope:', $null, $null)
    Add-Row Control @('FolderDialog', 'ScopeInfo', 'Text', 22, 120, 326, 16, $CTL_TEXT_PATH, $null,
        '[SCOPESUMMARY]', $null, $null)
    Add-Row Control @('FolderDialog', 'FolderHint', 'Text', 22, 142, 326, 24, $CTL_TEXT, $null,
        'Type any valid folder path directly above or click Browse to pick or create a folder on any drive.', $null, $null)
    Add-Row Control @('FolderDialog', 'FileAssocCheckBox', 'CheckBox', 22, 174, 326, 30, $CTL_BTN, 'REGISTERFILETYPES',
        '&Register Ecclesia PDF as a default app for supported file types (.pdf, .xps, .epub, .cbz, .cbr, .mobi, .fb2, .djvu, .expdf)', 'Back', $null)
    Add-Row CheckBox @('REGISTERFILETYPES', '1')
    Add-Row Control @('FolderDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN, $null, '< &Back', 'Next', $null)
    Add-Row Control @('FolderDialog', 'Next', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Next >', 'Cancel', $null)
    Add-Row Control @('FolderDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'FolderEdit', $null)

    Add-Row ControlEvent @('FolderDialog', 'Browse', '[_BrowseProperty]', 'INSTALLDIR', '1', 1)
    Add-Row ControlEvent @('FolderDialog', 'Browse', 'SpawnDialog', 'BrowseDialog', '1', 2)
    Add-Row ControlEvent @('FolderDialog', 'Back', 'NewDialog', 'ScopeDialog', '1', 1)
    Add-Row ControlEvent @('FolderDialog', 'Next', 'SetTargetPath', 'INSTALLDIR', '1', 1)
    Add-Row ControlEvent @('FolderDialog', 'Next', '[USERCUSTOMDIR]', '1', 'INSTALLDIR <> USERINSTALLDIR AND INSTALLDIR <> MACHINEINSTALLDIR', 2)
    Add-Row ControlEvent @('FolderDialog', 'Next', '[FILEASSOCSUMMARY]', 'Register for supported file types (.pdf, .epub, .xps, ...)', 'REGISTERFILETYPES = "1"', 3)
    Add-Row ControlEvent @('FolderDialog', 'Next', '[FILEASSOCSUMMARY]', 'Do not change file associations', 'REGISTERFILETYPES <> "1"', 4)
    Add-Row ControlEvent @('FolderDialog', 'Next', 'NewDialog', 'VerifyReadyDialog', '1', 5)
    Add-Row ControlEvent @('FolderDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 5. BrowseDialog (Standard MSI directory browser)
    # =========================================================================
    Add-Row Dialog @('BrowseDialog', 50, 50, 370, 270, $DLG_MODAL, $dlgTitle, 'PathEdit', 'OK', 'Cancel')
    Add-InteriorChrome 'BrowseDialog' 'Change Destination Folder' 'Browse to the folder where you want to install Ecclesia PDF.'
    Add-Row Control @('BrowseDialog', 'ComboLabel', 'Text', 22, 58, 44, 14, $CTL_TEXT, $null, '&Look in:', $null, $null)
    Add-Row Control @('BrowseDialog', 'DirectoryCombo', 'DirectoryCombo', 70, 55, 220, 80, $CTL_DIRCOMBO, '_BrowseProperty', $null, 'Up', $null)
    Add-Row Control @('BrowseDialog', 'Up', 'PushButton', 298, 55, 19, 19, $CTL_BTN_ICON16, $null, 'UpIco', 'NewFolder', 'Up one level|')
    Add-Row Control @('BrowseDialog', 'NewFolder', 'PushButton', 325, 55, 19, 19, $CTL_BTN_ICON16, $null, 'NewFolderIco', 'DirectoryList', 'Create a new folder|')
    Add-Row Control @('BrowseDialog', 'DirectoryList', 'DirectoryList', 22, 80, 326, 108, $CTL_DIRLIST, '_BrowseProperty', 'List of folders', 'PathEdit', $null)
    Add-Row Control @('BrowseDialog', 'PathLabel', 'Text', 22, 198, 60, 14, $CTL_TEXT, $null, '&Folder name:', $null, $null)
    Add-Row Control @('BrowseDialog', 'PathEdit', 'PathEdit', 84, 195, 264, 18, $CTL_PATHEDIT_IND, '_BrowseProperty', $null, 'OK', $null)
    Add-Row Control @('BrowseDialog', 'OK', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, 'OK', 'Cancel', $null)
    Add-Row Control @('BrowseDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'DirectoryCombo', $null)

    Add-Row ControlEvent @('BrowseDialog', 'Up', 'DirectoryListUp', '0', '1', 1)
    Add-Row ControlEvent @('BrowseDialog', 'NewFolder', 'DirectoryListNew', '0', '1', 1)
    Add-Row ControlEvent @('BrowseDialog', 'OK', 'SetTargetPath', '[_BrowseProperty]', '1', 1)
    Add-Row ControlEvent @('BrowseDialog', 'OK', 'EndDialog', 'Return', '1', 2)
    Add-Row ControlEvent @('BrowseDialog', 'Cancel', 'Reset', '0', '1', 1)
    Add-Row ControlEvent @('BrowseDialog', 'Cancel', 'EndDialog', 'Return', '1', 2)

    # =========================================================================
    # 6. VerifyReadyDialog (Summary + UAC ElevationShield when Per-Machine)
    # =========================================================================
    Add-Row Dialog @('VerifyReadyDialog', 50, 50, 370, 270, $DLG_WIZARD, $dlgTitle, 'Back', 'InstallPerUser', 'Cancel')
    Add-InteriorChrome 'VerifyReadyDialog' 'Ready to Install Ecclesia PDF' 'Review your installation choices and click Install to begin.'
    Add-Row Control @('VerifyReadyDialog', 'ScopeTitle', 'Text', 22, 56, 326, 14, $CTL_TEXT, $null,
        '{\DlgFontBold8}Installation scope:', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'ScopeValue', 'Text', 32, 70, 316, 16, $CTL_TEXT_PATH, $null,
        '[SCOPESUMMARY]', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'DirTitle', 'Text', 22, 92, 326, 14, $CTL_TEXT, $null,
        '{\DlgFontBold8}Destination folder:', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'DirValue', 'Text', 32, 106, 316, 24, $CTL_TEXT_PATH, $null,
        '[INSTALLDIR]', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'FileAssocTitle', 'Text', 22, 134, 326, 14, $CTL_TEXT, $null,
        '{\DlgFontBold8}File associations:', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'FileAssocValue', 'Text', 32, 148, 316, 18, $CTL_TEXT_PATH, $null,
        '[FILEASSOCSUMMARY]', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'ElevationNote', 'Text', 22, 172, 326, 24, $CTL_TEXT, $null,
        'Windows will prompt for administrator permission (UAC) when you click Install.', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'ReadyPrompt', 'Text', 22, 200, 326, 26, $CTL_TEXT, $null,
        'Click Install to begin the installation, Back to change any settings, or Cancel to exit.', $null, $null)
    Add-Row Control @('VerifyReadyDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN, $null, '< &Back', 'InstallPerUser', $null)
    Add-Row Control @('VerifyReadyDialog', 'InstallPerUser', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Install', 'InstallPerMachine', $null)
    Add-Row Control @('VerifyReadyDialog', 'InstallPerMachine', 'PushButton', 236, 243, 56, 17, $CTL_BTN_ELEVATED, $null, '&Install', 'Cancel', $null)
    Add-Row Control @('VerifyReadyDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'Back', $null)

    Add-Row ControlCondition @('VerifyReadyDialog', 'ElevationNote', 'Show', 'INSTALLSCOPE = "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'ElevationNote', 'Hide', 'INSTALLSCOPE <> "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerUser', 'Show', 'INSTALLSCOPE <> "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerUser', 'Hide', 'INSTALLSCOPE = "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerUser', 'Default', 'INSTALLSCOPE <> "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerMachine', 'Show', 'INSTALLSCOPE = "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerMachine', 'Hide', 'INSTALLSCOPE <> "PerMachine"')
    Add-Row ControlCondition @('VerifyReadyDialog', 'InstallPerMachine', 'Default', 'INSTALLSCOPE = "PerMachine"')

    Add-Row ControlEvent @('VerifyReadyDialog', 'Back', 'NewDialog', 'FolderDialog', '1', 1)
    Add-Row ControlEvent @('VerifyReadyDialog', 'InstallPerUser', 'EndDialog', 'Return', '1', 1)
    Add-Row ControlEvent @('VerifyReadyDialog', 'InstallPerMachine', 'EndDialog', 'Return', '1', 1)
    Add-Row ControlEvent @('VerifyReadyDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 7. MaintenanceDialog (Already installed: Repair or Remove)
    # =========================================================================
    Add-Row Dialog @('MaintenanceDialog', 50, 50, 370, 270, $DLG_WIZARD, $dlgTitle, 'Cancel', 'Cancel', 'Cancel')
    Add-InteriorChrome 'MaintenanceDialog' 'Repair or Remove EcclesiaPDF' 'EcclesiaPDF is already installed on this computer.'
    Add-Row Control @('MaintenanceDialog', 'RepairButton', 'PushButton', 22, 72, 88, 20, $CTL_BTN, $null, '&Repair', 'RepairButtonAdmin', $null)
    Add-Row Control @('MaintenanceDialog', 'RepairButtonAdmin', 'PushButton', 22, 72, 88, 20, $CTL_BTN_ELEVATED, $null, '&Repair', 'RemoveButton', $null)
    Add-Row Control @('MaintenanceDialog', 'RepairDesc', 'Text', 122, 72, 226, 36, $CTL_TEXT, $null,
        'Repairs errors in the current installation by restoring missing or damaged files and shortcuts.', $null, $null)
    Add-Row Control @('MaintenanceDialog', 'RemoveButton', 'PushButton', 22, 134, 88, 20, $CTL_BTN, $null, 'R&emove', 'RemoveButtonAdmin', $null)
    Add-Row Control @('MaintenanceDialog', 'RemoveButtonAdmin', 'PushButton', 22, 134, 88, 20, $CTL_BTN_ELEVATED, $null, 'R&emove', 'Cancel', $null)
    Add-Row Control @('MaintenanceDialog', 'RemoveDesc', 'Text', 122, 134, 226, 36, $CTL_TEXT, $null,
        'Removes Ecclesia PDF from your computer. Your documents, settings, and library data are kept.', $null, $null)
    Add-Row Control @('MaintenanceDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'RepairButton', $null)

    Add-Row ControlCondition @('MaintenanceDialog', 'RepairButton', 'Hide', 'ALLUSERS = "1" OR NOT MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RepairButton', 'Show', 'ALLUSERS <> "1" AND MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RepairButtonAdmin', 'Show', 'ALLUSERS = "1" OR NOT MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RepairButtonAdmin', 'Hide', 'ALLUSERS <> "1" AND MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RemoveButton', 'Hide', 'ALLUSERS = "1" OR NOT MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RemoveButton', 'Show', 'ALLUSERS <> "1" AND MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RemoveButtonAdmin', 'Show', 'ALLUSERS = "1" OR NOT MSIINSTALLPERUSER')
    Add-Row ControlCondition @('MaintenanceDialog', 'RemoveButtonAdmin', 'Hide', 'ALLUSERS <> "1" AND MSIINSTALLPERUSER')

    foreach ($btn in @('RepairButton', 'RepairButtonAdmin')) {
        Add-Row ControlEvent @('MaintenanceDialog', $btn, '[REINSTALLMODE]', 'ecmus', '1', 1)
        Add-Row ControlEvent @('MaintenanceDialog', $btn, '[REINSTALL]', 'ALL', '1', 2)
        Add-Row ControlEvent @('MaintenanceDialog', $btn, '[MAINTMODE]', 'Repair', '1', 3)
        Add-Row ControlEvent @('MaintenanceDialog', $btn, 'EndDialog', 'Return', '1', 4)
    }
    foreach ($btn in @('RemoveButton', 'RemoveButtonAdmin')) {
        Add-Row ControlEvent @('MaintenanceDialog', $btn, '[REMOVE]', 'ALL', '1', 1)
        Add-Row ControlEvent @('MaintenanceDialog', $btn, '[MAINTMODE]', 'Remove', '1', 2)
        Add-Row ControlEvent @('MaintenanceDialog', $btn, 'EndDialog', 'Return', '1', 3)
    }
    Add-Row ControlEvent @('MaintenanceDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 8. ProgressDialog (Modeless progress bar subscribed to ActionText / SetProgress)
    # =========================================================================
    Add-Row Dialog @('ProgressDialog', 50, 50, 370, 270, $DLG_MODELESS, $dlgTitle, 'Cancel', 'Cancel', 'Cancel')
    Add-Row Control @('ProgressDialog', 'BannerBitmap', 'Bitmap', 0, 0, 370, 44, $ATTR_VISIBLE, $null, 'TopBanner', $null, $null)
    Add-Row Control @('ProgressDialog', 'BannerLine', 'Line', 0, 44, 370, 0, $ATTR_VISIBLE, $null, $null, $null, $null)
    Add-Row Control @('ProgressDialog', 'TitleInstall', 'Text', 15, 6, 305, 15, $CTL_TEXT_BANNER, $null, '{\BannerTitleFont}Installing EcclesiaPDF', $null, $null)
    Add-Row Control @('ProgressDialog', 'DescInstall', 'Text', 15, 23, 305, 18, $CTL_TEXT_BANNER, $null, 'Please wait while EcclesiaPDF is installed.', $null, $null)
    Add-Row Control @('ProgressDialog', 'TitleRepair', 'Text', 15, 6, 305, 15, $CTL_TEXT_BANNER, $null, '{\BannerTitleFont}Repairing EcclesiaPDF', $null, $null)
    Add-Row Control @('ProgressDialog', 'DescRepair', 'Text', 15, 23, 305, 18, $CTL_TEXT_BANNER, $null, 'Please wait while EcclesiaPDF is repaired.', $null, $null)
    Add-Row Control @('ProgressDialog', 'TitleRemove', 'Text', 15, 6, 305, 15, $CTL_TEXT_BANNER, $null, '{\BannerTitleFont}Removing EcclesiaPDF', $null, $null)
    Add-Row Control @('ProgressDialog', 'DescRemove', 'Text', 15, 23, 305, 18, $CTL_TEXT_BANNER, $null, 'Please wait while EcclesiaPDF is removed.', $null, $null)
    Add-Row Control @('ProgressDialog', 'StatusLabel', 'Text', 22, 92, 44, 14, $CTL_TEXT, $null, 'Status:', $null, $null)
    Add-Row Control @('ProgressDialog', 'ActionText', 'Text', 68, 92, 280, 16, $CTL_TEXT_PATH, $null, $null, $null, $null)
    Add-Row Control @('ProgressDialog', 'ActionData', 'Text', 68, 110, 280, 16, $CTL_TEXT_PATH, $null, $null, $null, $null)
    Add-Row Control @('ProgressDialog', 'ProgressBar', 'ProgressBar', 22, 134, 326, 12, $CTL_PROGRESS, $null, 'Progress done', $null, $null)
    Add-Row Control @('ProgressDialog', 'BottomLine', 'Line', 0, 234, 370, 0, $ATTR_VISIBLE, $null, $null, $null, $null)
    Add-Row Control @('ProgressDialog', 'Back', 'PushButton', 172, 243, 56, 17, $CTL_BTN_DISABLED, $null, '< &Back', $null, $null)
    Add-Row Control @('ProgressDialog', 'Next', 'PushButton', 236, 243, 56, 17, $CTL_BTN_DISABLED, $null, '&Next >', $null, $null)
    Add-Row Control @('ProgressDialog', 'Cancel', 'PushButton', 301, 243, 56, 17, $CTL_BTN, $null, 'Cancel', 'Cancel', $null)

    Add-Row ControlCondition @('ProgressDialog', 'TitleInstall', 'Show', 'NOT Installed')
    Add-Row ControlCondition @('ProgressDialog', 'TitleInstall', 'Hide', 'Installed')
    Add-Row ControlCondition @('ProgressDialog', 'DescInstall', 'Show', 'NOT Installed')
    Add-Row ControlCondition @('ProgressDialog', 'DescInstall', 'Hide', 'Installed')
    Add-Row ControlCondition @('ProgressDialog', 'TitleRepair', 'Show', 'Installed AND MAINTMODE = "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'TitleRepair', 'Hide', 'NOT Installed OR MAINTMODE <> "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'DescRepair', 'Show', 'Installed AND MAINTMODE = "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'DescRepair', 'Hide', 'NOT Installed OR MAINTMODE <> "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'TitleRemove', 'Show', 'Installed AND MAINTMODE <> "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'TitleRemove', 'Hide', 'NOT Installed OR MAINTMODE = "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'DescRemove', 'Show', 'Installed AND MAINTMODE <> "Repair"')
    Add-Row ControlCondition @('ProgressDialog', 'DescRemove', 'Hide', 'NOT Installed OR MAINTMODE = "Repair"')

    Add-Row EventMapping @('ProgressDialog', 'ActionText', 'ActionText', 'Text')
    Add-Row EventMapping @('ProgressDialog', 'ActionData', 'ActionData', 'Text')
    Add-Row EventMapping @('ProgressDialog', 'ProgressBar', 'SetProgress', 'Progress')
    Add-Row ControlEvent @('ProgressDialog', 'Cancel', 'SpawnDialog', 'CancelDialog', '1', 1)

    # =========================================================================
    # 9. ExitDialog (Completion + optional Launch & Default Apps checkboxes)
    # =========================================================================
    Add-Row Dialog @('ExitDialog', 50, 50, 370, 270, $DLG_MODAL, $dlgTitle, 'Finish', 'Finish', 'Finish')
    Add-ExteriorChrome 'ExitDialog'
    Add-Row Control @('ExitDialog', 'Title', 'Text', 124, 20, 232, 32, $CTL_TEXT, $null,
        '{\DlgTitleFont}Ecclesia PDF Setup Complete', $null, $null)
    Add-Row Control @('ExitDialog', 'BodyInstall', 'Text', 124, 62, 232, 36, $CTL_TEXT, $null,
        'Ecclesia PDF [ProductVersion] has been installed on your computer. Click Finish to exit Setup.', $null, $null)
    Add-Row Control @('ExitDialog', 'BodyMaint', 'Text', 124, 62, 232, 36, $CTL_TEXT, $null,
        'Ecclesia PDF maintenance is complete. Click Finish to exit Setup.', $null, $null)
    Add-Row Control @('ExitDialog', 'LaunchCheckBox', 'CheckBox', 124, 114, 224, 18, $CTL_BTN, 'LAUNCHAPP',
        '&Launch Ecclesia PDF', 'DefaultAppsCheckBox', $null)
    Add-Row Control @('ExitDialog', 'DefaultAppsCheckBox', 'CheckBox', 124, 138, 224, 28, $CTL_BTN, 'OPENDEFAULTAPPS',
        '&Open Windows Default Apps settings to set Ecclesia PDF as default', 'Finish', $null)
    Add-Row Control @('ExitDialog', 'Finish', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Finish', 'LaunchCheckBox', $null)
    Add-Row CheckBox @('LAUNCHAPP', '1')
    Add-Row CheckBox @('OPENDEFAULTAPPS', '1')

    Add-Row ControlCondition @('ExitDialog', 'BodyInstall', 'Show', 'NOT Installed')
    Add-Row ControlCondition @('ExitDialog', 'BodyInstall', 'Hide', 'Installed')
    Add-Row ControlCondition @('ExitDialog', 'BodyMaint', 'Show', 'Installed')
    Add-Row ControlCondition @('ExitDialog', 'BodyMaint', 'Hide', 'NOT Installed')
    Add-Row ControlCondition @('ExitDialog', 'LaunchCheckBox', 'Show', 'NOT Installed')
    Add-Row ControlCondition @('ExitDialog', 'LaunchCheckBox', 'Hide', 'Installed')
    Add-Row ControlCondition @('ExitDialog', 'DefaultAppsCheckBox', 'Show', 'NOT Installed AND REGISTERFILETYPES = "1"')
    Add-Row ControlCondition @('ExitDialog', 'DefaultAppsCheckBox', 'Hide', 'Installed OR REGISTERFILETYPES <> "1"')

    Add-Row ControlEvent @('ExitDialog', 'Finish', 'DoAction', 'OpenDefaultApps', 'OPENDEFAULTAPPS = "1" AND REGISTERFILETYPES = "1" AND NOT Installed', 1)
    Add-Row ControlEvent @('ExitDialog', 'Finish', 'DoAction', 'LaunchApp', 'LAUNCHAPP = "1" AND NOT Installed', 2)
    Add-Row ControlEvent @('ExitDialog', 'Finish', 'EndDialog', 'Return', '1', 3)

    # =========================================================================
    # 10. CancelDialog (Cancel confirmation)
    # =========================================================================
    Add-Row Dialog @('CancelDialog', 50, 50, 260, 85, $DLG_MODAL, $dlgTitle, 'No', 'No', 'No')
    Add-Row Control @('CancelDialog', 'Icon', 'Icon', 15, 15, 24, 24, $CTL_ICON_32, $null, 'AppIconBin', $null, $null)
    Add-Row Control @('CancelDialog', 'Text', 'Text', 48, 15, 194, 34, $CTL_TEXT, $null,
        'Are you sure you want to cancel Ecclesia PDF Setup?', $null, $null)
    Add-Row Control @('CancelDialog', 'Yes', 'PushButton', 72, 57, 56, 17, $CTL_BTN, $null, '&Yes', 'No', $null)
    Add-Row Control @('CancelDialog', 'No', 'PushButton', 132, 57, 56, 17, $CTL_BTN, $null, '&No', 'Yes', $null)
    Add-Row ControlEvent @('CancelDialog', 'Yes', 'EndDialog', 'Exit', '1', 1)
    Add-Row ControlEvent @('CancelDialog', 'No', 'EndDialog', 'Return', '1', 1)

    # =========================================================================
    # 11. UserExitDialog (Sequence -2)
    # =========================================================================
    Add-Row Dialog @('UserExitDialog', 50, 50, 370, 270, $DLG_MODAL, $dlgTitle, 'Finish', 'Finish', 'Finish')
    Add-ExteriorChrome 'UserExitDialog'
    Add-Row Control @('UserExitDialog', 'Title', 'Text', 124, 20, 232, 32, $CTL_TEXT, $null,
        '{\DlgTitleFont}Ecclesia PDF Setup Cancelled', $null, $null)
    Add-Row Control @('UserExitDialog', 'Body', 'Text', 124, 62, 232, 48, $CTL_TEXT, $null,
        'Ecclesia PDF Setup was interrupted. Your system has not been modified. Run Setup again at any time to complete the installation.', $null, $null)
    Add-Row Control @('UserExitDialog', 'Finish', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Finish', 'Finish', $null)
    Add-Row ControlEvent @('UserExitDialog', 'Finish', 'EndDialog', 'Exit', '1', 1)

    # =========================================================================
    # 12. FatalErrorDialog (Sequence -3)
    # =========================================================================
    Add-Row Dialog @('FatalErrorDialog', 50, 50, 370, 270, $DLG_MODAL, $dlgTitle, 'Finish', 'Finish', 'Finish')
    Add-ExteriorChrome 'FatalErrorDialog'
    Add-Row Control @('FatalErrorDialog', 'Title', 'Text', 124, 20, 232, 32, $CTL_TEXT, $null,
        '{\DlgTitleFont}Setup Ended Prematurely', $null, $null)
    Add-Row Control @('FatalErrorDialog', 'Body', 'Text', 124, 62, 232, 56, $CTL_TEXT, $null,
        'Ecclesia PDF Setup ended prematurely because of an error. Your system has not been modified. Run Setup again later to install Ecclesia PDF.', $null, $null)
    Add-Row Control @('FatalErrorDialog', 'Finish', 'PushButton', 236, 243, 56, 17, $CTL_BTN, $null, '&Finish', 'Finish', $null)
    Add-Row ControlEvent @('FatalErrorDialog', 'Finish', 'EndDialog', 'Exit', '1', 1)

    # =========================================================================
    # 13. ErrorDialog (Standard MSI error message box; Property ErrorDialog='ErrorDialog')
    # =========================================================================
    Add-Row Dialog @('ErrorDialog', 50, 50, 270, 105, $DLG_ERROR, $dlgTitle, 'ErrorText', $null, $null)
    Add-Row Control @('ErrorDialog', 'ErrorIcon', 'Icon', 15, 15, 24, 24, $CTL_ICON_32, $null, 'AppIconBin', $null, $null)
    Add-Row Control @('ErrorDialog', 'ErrorText', 'Text', 48, 15, 205, 60, $CTL_TEXT, $null, 'Information text', $null, $null)
    Add-Row Control @('ErrorDialog', 'O', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&OK', $null, $null)
    Add-Row Control @('ErrorDialog', 'C', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&Cancel', $null, $null)
    Add-Row Control @('ErrorDialog', 'Y', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&Yes', $null, $null)
    Add-Row Control @('ErrorDialog', 'N', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&No', $null, $null)
    Add-Row Control @('ErrorDialog', 'R', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&Retry', $null, $null)
    Add-Row Control @('ErrorDialog', 'I', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&Ignore', $null, $null)
    Add-Row Control @('ErrorDialog', 'A', 'PushButton', 100, 80, 56, 17, $CTL_BTN, $null, '&Abort', $null, $null)
    Add-Row ControlEvent @('ErrorDialog', 'O', 'EndDialog', 'ErrorOk', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'C', 'EndDialog', 'ErrorCancel', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'Y', 'EndDialog', 'ErrorYes', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'N', 'EndDialog', 'ErrorNo', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'R', 'EndDialog', 'ErrorRetry', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'I', 'EndDialog', 'ErrorIgnore', '1', 1)
    Add-Row ControlEvent @('ErrorDialog', 'A', 'EndDialog', 'ErrorAbort', '1', 1)

    # --- UIText, ActionText, Error tables ---
    $uiTexts = [ordered] @{
        NewFolder            = 'Fldr|New Folder'
        bytes                = 'bytes'
        KB                   = 'KB'
        MB                   = 'MB'
        GB                   = 'GB'
        TimeRemaining        = 'Time remaining: {[1] minutes }{[2] seconds}'
        VolumeCostAvailable  = 'Available'
        VolumeCostDifference = 'Difference'
        VolumeCostRequired   = 'Required'
        VolumeCostSize       = 'Disk Size'
        VolumeCostVolume     = 'Volume'
    }
    foreach ($k in $uiTexts.Keys) { Add-Row UIText @($k, $uiTexts[$k]) }

    Add-Row ActionText @('CostInitialize',         'Computing space requirements',            $null)
    Add-Row ActionText @('FileCost',               'Computing space requirements',            $null)
    Add-Row ActionText @('CostFinalize',           'Computing space requirements',            $null)
    Add-Row ActionText @('StopRunningApp',         'Closing running instances of Ecclesia PDF', $null)
    Add-Row ActionText @('InstallValidate',        'Validating install',                      $null)
    Add-Row ActionText @('InstallInitialize',      'Preparing installation',                  $null)
    Add-Row ActionText @('RemoveExistingProducts', 'Removing older versions of EcclesiaPDF', 'Application: [1]')
    Add-Row ActionText @('ProcessComponents',      'Updating component registration',         $null)
    Add-Row ActionText @('UnpublishFeatures',      'Unpublishing product features',           'Feature: [1]')
    Add-Row ActionText @('RemoveRegistryValues',   'Removing registry values',                'Key: [1], Name: [2]')
    Add-Row ActionText @('RemoveShortcuts',        'Removing shortcuts',                      'Shortcut: [1]')
    Add-Row ActionText @('RemoveFiles',            'Removing files',                          'File: [1]')
    Add-Row ActionText @('InstallFiles',           'Copying new files',                       'File: [1]')
    Add-Row ActionText @('CreateShortcuts',        'Creating shortcuts',                      'Shortcut: [1]')
    Add-Row ActionText @('WriteRegistryValues',    'Writing system registry values',          'Key: [1], Name: [2]')
    Add-Row ActionText @('RegisterUser',           'Registering user',                        '[1]')
    Add-Row ActionText @('RegisterProduct',        'Registering product',                     '[1]')
    Add-Row ActionText @('PublishFeatures',        'Publishing product features',             'Feature: [1]')
    Add-Row ActionText @('PublishProduct',         'Publishing product information',          $null)
    Add-Row ActionText @('InstallFinalize',        'Finalizing installation',                 $null)
    Add-Row ActionText @('Rollback',               'Rolling back action:',                    '[1]')
    Add-Row ActionText @('RollbackCleanup',        'Removing backup files',                   'File: [1]')

    $errors = [ordered] @{
        0    = '{{Fatal error: }}'
        1    = '{{Error [1]. }}'
        2    = 'Warning [1]. '
        4    = 'Info [1]. '
        5    = 'The installer has encountered an unexpected error installing this package. The error code is [1]. {{Arguments: [2], [3], [4]}}'
        1101 = 'Error reading from file: [2]. Verify that the file exists and that you can access it.'
        1303 = 'The installer has insufficient privileges to access this directory: [2]. Choose another folder or run as administrator.'
        1304 = 'Error writing to file: [2]. Verify that you have access to that directory.'
        1309 = 'Error reading from file: [3]. Verify that the file exists and that you can access it.'
        1310 = 'Error writing to file: [3]. Verify that you have access to that directory.'
        1314 = "The specified path '[2]' is unavailable."
        1316 = 'A network error occurred while attempting to read from the file: [2].'
        1321 = 'The installer has insufficient privileges to modify this file: [2].'
        1324 = "The folder path '[2]' contains an invalid character."
        1325 = "'[2]' is not a valid short file name."
        1327 = 'Invalid Drive: [2]'
        1606 = 'Could not access network location [2].'
        1607 = 'The following applications should be closed before continuing the install:'
        1706 = 'An installation package for the product [2] cannot be found.'
        1707 = 'Installation completed successfully.'
        1708 = 'Installation failed.'
        1709 = 'Product: [2] -- [3]'
        1719 = 'The Windows Installer Service could not be accessed.'
        1721 = 'There is a problem with this Windows Installer package.'
        1925 = 'You do not have sufficient privileges to complete this installation for all users. Log on as administrator or choose a current-user installation.'
        1926 = "Could not set file security for file '[3]'. Verify that you have sufficient privileges."
    }
    foreach ($entry in $errors.GetEnumerator()) { Add-Row Error @([int]$entry.Key, $entry.Value) }

    # --- Sequences ---
    Add-Row InstallExecuteSequence @('FindRelatedProducts',    $null, 25)
    Add-Row InstallExecuteSequence @('MarkCustomDir',          'INSTALLDIR', 50)
    Add-Row InstallExecuteSequence @('LaunchConditions',       $null, 100)
    Add-Row InstallExecuteSequence @('ValidateProductID',      $null, 700)
    Add-Row InstallExecuteSequence @('CostInitialize',         $null, 800)
    Add-Row InstallExecuteSequence @('FileCost',               $null, 900)
    Add-Row InstallExecuteSequence @('CostFinalize',           $null, 1000)
    Add-Row InstallExecuteSequence @('InitUserDir',            'NOT USERINSTALLDIR', 1010)
    Add-Row InstallExecuteSequence @('InitMachineDir',         'NOT MACHINEINSTALLDIR AND %ProgramW6432', 1011)
    Add-Row InstallExecuteSequence @('InitMachineDirFallback', 'NOT MACHINEINSTALLDIR', 1012)
    Add-Row InstallExecuteSequence @('SetInstallDirMachine',   'NOT Installed AND NOT USERCUSTOMDIR AND (ALLUSERS = "1" OR NOT MSIINSTALLPERUSER)', 1025)
    Add-Row InstallExecuteSequence @('StopRunningApp',         $null, 1350)
    Add-Row InstallExecuteSequence @('InstallValidate',        $null, 1400)
    Add-Row InstallExecuteSequence @('InstallInitialize',      $null, 1500)
    Add-Row InstallExecuteSequence @('RemoveExistingProducts', $null, 1501)
    Add-Row InstallExecuteSequence @('ProcessComponents',      $null, 1600)
    Add-Row InstallExecuteSequence @('UnpublishFeatures',      $null, 1800)
    Add-Row InstallExecuteSequence @('RemoveRegistryValues',   $null, 2600)
    Add-Row InstallExecuteSequence @('RemoveShortcuts',        $null, 3200)
    Add-Row InstallExecuteSequence @('RemoveFiles',            $null, 3500)
    Add-Row InstallExecuteSequence @('InstallFiles',           $null, 4000)
    Add-Row InstallExecuteSequence @('CreateShortcuts',        $null, 4500)
    Add-Row InstallExecuteSequence @('WriteRegistryValues',    $null, 5000)
    Add-Row InstallExecuteSequence @('RegisterUser',           $null, 6000)
    Add-Row InstallExecuteSequence @('RegisterProduct',        $null, 6100)
    Add-Row InstallExecuteSequence @('PublishFeatures',        $null, 6300)
    Add-Row InstallExecuteSequence @('PublishProduct',         $null, 6400)
    Add-Row InstallExecuteSequence @('InstallFinalize',        $null, 6600)

    Add-Row InstallUISequence @('FatalErrorDialog',       $null, -3)
    Add-Row InstallUISequence @('UserExitDialog',         $null, -2)
    Add-Row InstallUISequence @('ExitDialog',             $null, -1)
    Add-Row InstallUISequence @('FindRelatedProducts',    $null, 25)
    Add-Row InstallUISequence @('MarkCustomDir',          'INSTALLDIR', 50)
    Add-Row InstallUISequence @('LaunchConditions',       $null, 100)
    Add-Row InstallUISequence @('PrepareDialog',          $null, 140)
    Add-Row InstallUISequence @('ValidateProductID',      $null, 700)
    Add-Row InstallUISequence @('CostInitialize',         $null, 800)
    Add-Row InstallUISequence @('FileCost',               $null, 900)
    Add-Row InstallUISequence @('CostFinalize',           $null, 1000)
    Add-Row InstallUISequence @('InitUserDir',            'NOT USERINSTALLDIR', 1010)
    Add-Row InstallUISequence @('InitMachineDir',         'NOT MACHINEINSTALLDIR AND %ProgramW6432', 1011)
    Add-Row InstallUISequence @('InitMachineDirFallback', 'NOT MACHINEINSTALLDIR', 1012)
    Add-Row InstallUISequence @('SyncScopeMachine',       'NOT Installed AND (ALLUSERS = "1" OR NOT MSIINSTALLPERUSER)', 1020)
    Add-Row InstallUISequence @('SetScopeSummaryUser',    'INSTALLSCOPE <> "PerMachine"', 1021)
    Add-Row InstallUISequence @('SetScopeSummaryMachine', 'INSTALLSCOPE = "PerMachine"', 1022)
    Add-Row InstallUISequence @('SetInstallDirMachine',   'NOT Installed AND NOT USERCUSTOMDIR AND (ALLUSERS = "1" OR NOT MSIINSTALLPERUSER)', 1025)
    Add-Row InstallUISequence @('WelcomeDialog',          'NOT Installed', 1200)
    Add-Row InstallUISequence @('MaintenanceDialog',      'Installed AND NOT RESUME AND NOT Preselected', 1205)
    Add-Row InstallUISequence @('ProgressDialog',         $null, 1290)
    Add-Row InstallUISequence @('ExecuteAction',          $null, 1300)

    # An administrative install (msiexec /a) unpacks the files: how tests verify the package payload.
    $admin = [ordered] @{
        CostInitialize = 800; FileCost = 900; CostFinalize = 1000; InstallValidate = 1400
        InstallInitialize = 1500; InstallAdminPackage = 3900; InstallFiles = 4000; InstallFinalize = 6600
    }
    foreach ($action in $admin.Keys) { Add-Row AdminExecuteSequence @($action, $null, $admin[$action]) }
    $adminUi = [ordered] @{
        CostInitialize = 800; FileCost = 900; CostFinalize = 1000; ExecuteAction = 1300
    }
    foreach ($action in $adminUi.Keys) { Add-Row AdminUISequence @($action, $null, $adminUi[$action]) }

    # --- Binary & Icon Streams ---
    # Icon table: shown in Settings > Apps (ARPPRODUCTICON); shortcut takes the icon embedded in ecclesiapdf.exe.
    Add-Stream 'Icon' 'AppIcon.ico' $appIconPath
    # Binary table: used by MSI UI Bitmap, Icon, and PushButton(Icon) controls.
    Add-Stream 'Binary' 'AppIconBin' $appIcon32Path
    Add-Stream 'Binary' 'BannerSide' $bannerSidePath
    Add-Stream 'Binary' 'TopBanner' $topBannerPath
    Add-Stream 'Binary' 'UpIco' $upIcoPath
    Add-Stream 'Binary' 'NewFolderIco' $newFolderIcoPath
    # Embedded compressed cabinet stream.
    Add-Stream '_Streams' 'product.cab' $cab

    # --- Summary Information Stream (MSI 5.0 dual-purpose UAC-compliant) ---
    $summary = Get-ComProperty $db 'SummaryInformation' @(20)
    $fields = [ordered] @{
        1  = [int] 1252                       # codepage
        2  = 'Installation Database'
        3  = 'EcclesiaPDF'
        4  = 'EcclesiaPDF'
        5  = 'Installer'
        6  = 'EcclesiaPDF - Dual-purpose per-user / per-machine Windows installer'
        7  = 'x64;1033'                       # platform and language
        9  = "{$([guid]::NewGuid().ToString().ToUpper())}" # package code, new for every build
        12 = [datetime]::Now
        13 = [datetime]::Now
        14 = [int] 500                        # Windows Installer 5.0: required for x64 and Single Package Authoring
        15 = [int] 2                          # 2: compressed files; bit 3 clear so All Users can trigger UAC under Single Package Authoring
        18 = 'EcclesiaPDF build script'
    }
    foreach ($field in $fields.GetEnumerator()) { Set-ComProperty $summary 'Property' @([int] $field.Key, $field.Value) }
    [void] (Invoke-Com $summary 'Persist')
    [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($summary)
    [void] (Invoke-Com $db 'Commit')
    [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($db)
    [void] [Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $work
}

Get-Item $msi | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }, FullName
