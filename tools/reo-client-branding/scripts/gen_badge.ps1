# Generate the ReoPlus in-game badge font and metadata from the chrome R mark.
#
# Outputs (relative to -OutDir, default ../assets):
#   badge.png             64x36 approved tab-list texture (16x9 at 4x); preserved
#   badge.png.mcmeta      nearest-neighbour metadata for the tab texture
#   badge_font.png        24x24 anti-aliased chrome R font atlas
#   badge_font.png.mcmeta linear filtering for the 12px glyph
#   badge.json            one \uE000 glyph at height 12, ascent 11
#
# The 64x36 tab texture is a checked-in design asset (assets/badge.png), not a
# 9x8 mask upscale.  The nametag glyph is rendered into a 24x24 cell with a 2px
# margin and a 2px dark rim; Minecraft draws it at 12 logical pixels, which maps
# 1:1 at GUI scale 2 and stays smooth at scale 3+.
param(
  [string]$OutDir = "",
  [string]$SourcePng = ""
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolkitDir = Split-Path -Parent $scriptDir
if (-not $OutDir) { $OutDir = Join-Path $toolkitDir 'assets' }
if (-not $SourcePng) {
  $SourcePng = Join-Path (Split-Path -Parent (Split-Path -Parent $toolkitDir)) 'packages\reo_app\icons\Chrome R Lettermark Logo.png'
}
if (-not (Test-Path -LiteralPath $SourcePng)) { throw "source mark missing: $SourcePng" }
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null

# The approved tab texture is a design asset.  When generating into another
# directory, copy it across; never replace it with a 9x8 mask upscale.
$assetTab = Join-Path $toolkitDir 'assets\badge.png'
$assetTabMeta = Join-Path $toolkitDir 'assets\badge.png.mcmeta'
$tabPath = Join-Path $OutDir 'badge.png'
$tabMetaPath = Join-Path $OutDir 'badge.png.mcmeta'
if ((Resolve-Path -LiteralPath $OutDir).Path -ne (Resolve-Path -LiteralPath (Join-Path $toolkitDir 'assets')).Path) {
  if (-not (Test-Path -LiteralPath $assetTab)) { throw "approved tab texture missing: $assetTab" }
  Copy-Item -LiteralPath $assetTab -Destination $tabPath -Force
  Copy-Item -LiteralPath $assetTabMeta -Destination $tabMetaPath -Force
}
if (-not (Test-Path -LiteralPath $tabPath)) { throw "approved tab texture missing: $tabPath" }
if (-not (Test-Path -LiteralPath $tabMetaPath)) { throw "approved tab metadata missing: $tabMetaPath" }

Add-Type -ReferencedAssemblies 'System.Drawing' -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;

public static class ReoBadgeFont
{
    static Bitmap CropAlpha(Bitmap src)
    {
        int w = src.Width, h = src.Height;
        int minX = w, minY = h, maxX = -1, maxY = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            if (src.GetPixel(x, y).A > 8) {
                if (x < minX) minX = x; if (y < minY) minY = y;
                if (x > maxX) maxX = x; if (y > maxY) maxY = y;
            }
        }
        if (maxX < minX) return (Bitmap)src.Clone();
        var rect = Rectangle.FromLTRB(minX, minY, maxX + 1, maxY + 1);
        var dst = new Bitmap(rect.Width, rect.Height, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(dst)) {
            g.DrawImage(src, new Rectangle(0, 0, rect.Width, rect.Height), rect, GraphicsUnit.Pixel);
        }
        return dst;
    }

    static void AddRim(Bitmap bmp, Color color, int radius, int threshold) {
        int w = bmp.Width, h = bmp.Height;
        bool[] solid = new bool[w * h];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) solid[y*w+x] = bmp.GetPixel(x,y).A > threshold;
        bool[] rim = new bool[w * h];
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                if (solid[y*w+x]) continue;
                bool near = false;
                for (int dy = -radius; dy <= radius && !near; dy++) {
                    for (int dx = -radius; dx <= radius && !near; dx++) {
                        if (dx*dx + dy*dy > radius*radius) continue;
                        int nx = x + dx, ny = y + dy;
                        if (nx >= 0 && ny >= 0 && nx < w && ny < h && solid[ny*w+nx]) near = true;
                    }
                }
                if (near) rim[y*w+x] = true;
            }
        }
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) if (rim[y*w+x]) bmp.SetPixel(x,y,color);
    }

    public static void Make(string source, string output, int size, int margin, int rimRadius) {
        using (var src = new Bitmap(source))
        using (var cut = CropAlpha(src)) {
            var cell = new Bitmap(size, size, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(cell)) {
                g.Clear(Color.Transparent);
                int inner = size - margin * 2;
                int drawW = inner, drawH = inner;
                double ar = (double)cut.Width / cut.Height;
                if (ar > 1) drawH = Math.Max(1, (int)Math.Round(inner / ar));
                else drawW = Math.Max(1, (int)Math.Round(inner * ar));
                int dx = (size - drawW) / 2, dy = (size - drawH) / 2;
                g.InterpolationMode = InterpolationMode.HighQualityBicubic;
                g.PixelOffsetMode = PixelOffsetMode.HighQuality;
                g.CompositingMode = CompositingMode.SourceOver;
                g.DrawImage(cut, new Rectangle(dx, dy, drawW, drawH), 0, 0, cut.Width, cut.Height, GraphicsUnit.Pixel);
            }
            if (rimRadius > 0) AddRim(cell, Color.FromArgb(255, 5, 10, 23), rimRadius, 96);
            Directory.CreateDirectory(Path.GetDirectoryName(output));
            cell.Save(output, ImageFormat.Png);
            cell.Dispose();
        }
    }
}
'@

$fontPath = Join-Path $OutDir 'badge_font.png'
[ReoBadgeFont]::Make($SourcePng, $fontPath, 24, 2, 2)

$json = @'
{
  "providers": [
    {
      "type": "bitmap",
      "file": "polyplus:badge_font.png",
      "ascent": 11,
      "height": 12,
      "chars": ["\uE000"]
    },
    {
      "type": "space",
      "advances": {
        "\uE001": 3
      }
    }
  ]
}
'@
[System.IO.File]::WriteAllText((Join-Path $OutDir 'badge.json'), $json, (New-Object System.Text.UTF8Encoding($false)))

$mcmeta = @'
{
  "texture": {
    "blur": true,
    "clamp": true
  }
}
'@
[System.IO.File]::WriteAllText((Join-Path $OutDir 'badge_font.png.mcmeta'), $mcmeta, (New-Object System.Text.UTF8Encoding($false)))

$font = [System.Drawing.Image]::FromFile($fontPath)
"font: $($font.Width)x$($font.Height)"
$font.Dispose()
Get-FileHash -Algorithm SHA1 -LiteralPath $tabPath, $tabMetaPath, $fontPath, (Join-Path $OutDir 'badge_font.png.mcmeta'), (Join-Path $OutDir 'badge.json') |
  Select-Object Path, Hash |
  Format-Table -AutoSize
