# Draws Charpick's icons (PNG-compressed .ico files) into ..\assets.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tools\make-icons.ps1

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$assets = Join-Path $PSScriptRoot '..\assets'
New-Item -ItemType Directory -Force $assets | Out-Null

$glyph = [string][char]0x00E0   # à

function New-GlyphPath([float]$size, [float]$box, [string]$family) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $fam = New-Object System.Drawing.FontFamily($family)
    $path.AddString($glyph, $fam, [int][System.Drawing.FontStyle]::Bold, 100, (New-Object System.Drawing.PointF(0, 0)),
        [System.Drawing.StringFormat]::GenericTypographic)
    # Scale the glyph to fit `box` and center it in the icon.
    $b = $path.GetBounds()
    $k = [Math]::Min($box / $b.Width, $box / $b.Height)
    $m = New-Object System.Drawing.Drawing2D.Matrix
    $m.Translate(($size - $b.Width * $k) / 2, ($size - $b.Height * $k) / 2)
    $m.Scale($k, $k)
    $m.Translate(-$b.X, -$b.Y)
    $path.Transform($m)
    return $path
}

function New-RoundedRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = 2 * $r
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function Get-Png([int]$size, [scriptblock]$draw) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.CompositingQuality = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)
    & $draw $g $size
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return , $ms.ToArray()
}

function Write-Ico([string]$file, [int[]]$sizes, [scriptblock]$draw) {
    $images = @()
    foreach ($s in $sizes) { $images += , (Get-Png $s $draw) }
    $out = New-Object System.IO.MemoryStream
    $w = New-Object System.IO.BinaryWriter($out)
    $w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $s = $sizes[$i]
        $dim = if ($s -ge 256) { 0 } else { $s }
        $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
        $w.Write([uint16]1); $w.Write([uint16]32)
        $w.Write([uint32]$images[$i].Length); $w.Write([uint32]$offset)
        $offset += $images[$i].Length
    }
    foreach ($img in $images) { $w.Write($img) }
    $w.Flush()
    [System.IO.File]::WriteAllBytes((Join-Path $assets $file), $out.ToArray())
    Write-Host "wrote $file"
}

$family = 'Segoe UI'

# Application icon: a blue rounded square with a white letter.
Write-Ico 'charpick.ico' @(16, 20, 24, 32, 40, 48, 64, 96, 128, 256) {
    param($g, $size)
    $inset = [Math]::Max(0.5, $size * 0.04)
    $side = $size - 2 * $inset
    $rect = New-RoundedRect $inset $inset $side $side ($size * 0.2)
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
        (New-Object System.Drawing.PointF(0, 0)), (New-Object System.Drawing.PointF($size, $size)),
        [System.Drawing.Color]::FromArgb(255, 0x3A, 0x96, 0xDD), [System.Drawing.Color]::FromArgb(255, 0x00, 0x5A, 0x9E))
    $g.FillPath($brush, $rect)
    $path = New-GlyphPath $size ($size * 0.6) $family
    $g.FillPath([System.Drawing.Brushes]::White, $path)
}

# Tray icons: just the letter, light for dark taskbars and dark for light ones.
foreach ($v in @(@{ File = 'tray-light.ico'; Color = [System.Drawing.Color]::White },
                 @{ File = 'tray-dark.ico'; Color = [System.Drawing.Color]::FromArgb(255, 0x1C, 0x1C, 0x1C) })) {
    $color = $v.Color
    Write-Ico $v.File @(16, 20, 24, 32, 40, 48, 64) {
        param($g, $size)
        $path = New-GlyphPath $size ($size * 0.94) $family
        $g.FillPath((New-Object System.Drawing.SolidBrush($color)), $path)
    }
}
