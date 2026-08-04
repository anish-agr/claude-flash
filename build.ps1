# Builds bin\flash.exe from src\Flash.cs.
#
# Uses the C# compiler that ships with Windows (.NET Framework 4), so there is
# nothing to install.

[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$src = Join-Path $root 'src\Flash.cs'
$binDir = Join-Path $root 'bin'
$exe = Join-Path $binDir 'flash.exe'
$assets = Join-Path $root 'assets'
$ico = Join-Path $assets 'flash.ico'

$csc = @(
    "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
    "$env:WINDIR\Microsoft.NET\Framework\v4.0.30319\csc.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $csc) { throw "Could not find csc.exe (.NET Framework 4). Is .NET Framework installed?" }

New-Item -ItemType Directory -Force -Path $binDir, $assets | Out-Null

# ---- icon ------------------------------------------------------------------
# A green orb, so the desktop shortcut is obvious at a glance. Written as an
# ICO containing PNG frames (supported since Vista).

function New-FlashIcon {
    param([string]$Path)

    Add-Type -AssemblyName System.Drawing

    $frames = @()
    foreach ($size in 256, 64, 32) {
        $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.Clear([System.Drawing.Color]::Transparent)

        $pad = [Math]::Max(1, [int]($size * 0.06))
        $d = $size - (2 * $pad)
        $rect = New-Object System.Drawing.Rectangle($pad, $pad, $d, $d)

        $orb = New-Object System.Drawing.Drawing2D.GraphicsPath
        $orb.AddEllipse($rect)
        $brush = New-Object System.Drawing.Drawing2D.PathGradientBrush($orb)
        $brush.CenterPoint = New-Object System.Drawing.PointF(($size * 0.42), ($size * 0.36))
        $brush.CenterColor = [System.Drawing.Color]::FromArgb(255, 190, 255, 214)
        $brush.SurroundColors = @([System.Drawing.Color]::FromArgb(255, 0, 176, 74))
        $g.FillPath($brush, $orb)

        $penWidth = [Math]::Max(1.0, $size * 0.035)
        $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(235, 0, 255, 106), $penWidth)
        $g.DrawEllipse($pen, $rect)

        $pen.Dispose(); $brush.Dispose(); $orb.Dispose(); $g.Dispose()

        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $frames += , @{ Size = $size; Bytes = $ms.ToArray() }
        $ms.Dispose()
    }

    $fs = [System.IO.File]::Create($Path)
    $bw = New-Object System.IO.BinaryWriter($fs)
    try {
        $bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$frames.Count)
        $offset = 6 + (16 * $frames.Count)
        foreach ($f in $frames) {
            $dim = if ($f.Size -ge 256) { 0 } else { $f.Size }
            $bw.Write([Byte]$dim); $bw.Write([Byte]$dim)
            $bw.Write([Byte]0); $bw.Write([Byte]0)
            $bw.Write([UInt16]1); $bw.Write([UInt16]32)
            $bw.Write([UInt32]$f.Bytes.Length); $bw.Write([UInt32]$offset)
            $offset += $f.Bytes.Length
        }
        foreach ($f in $frames) { $bw.Write($f.Bytes) }
    } finally { $bw.Dispose(); $fs.Dispose() }
}

if ($Force -or -not (Test-Path $ico)) {
    Write-Host "Generating icon..." -ForegroundColor DarkGray
    New-FlashIcon -Path $ico
}

# ---- compile ---------------------------------------------------------------

Write-Host "Compiling flash.exe..." -ForegroundColor DarkGray

$cscArgs = @(
    '/nologo'
    '/target:winexe'
    '/optimize+'
    '/platform:anycpu'
    "/out:$exe"
    "/win32icon:$ico"
    '/reference:System.dll'
    '/reference:System.Drawing.dll'
    '/reference:System.Windows.Forms.dll'
    $src
)

$output = & $csc $cscArgs
if ($LASTEXITCODE -ne 0) {
    $output | ForEach-Object { Write-Host $_ -ForegroundColor Red }
    throw "Compilation failed."
}
$output | Where-Object { $_ -match '\S' } | ForEach-Object { Write-Host $_ -ForegroundColor Yellow }

Write-Host "Built $exe" -ForegroundColor Green
