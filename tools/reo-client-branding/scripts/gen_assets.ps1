param(
  [string]$Base = "C:\Users\drako\AppData\Local\Temp\opencode\reo_client_patch"
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName PresentationCore, WindowsBase

$inv = [System.Globalization.CultureInfo]::InvariantCulture
function Fmt([double]$v) { $v.ToString('0.##', $inv) }

# ============ 1. R mark from traced loops ============
$json = Get-Content "$Base\assets\loops.json" -Raw | ConvertFrom-Json
$mw = [double]$json.w; $mh = [double]$json.h

function Build-RPaths($rawLoops, [double]$s, [double]$tx, [double]$ty) {
  $out = @()
  foreach ($l in $rawLoops) {
    $sb = New-Object System.Text.StringBuilder
    $first = $true
    foreach ($p in $l) {
      $x = [double]$p[0] * $s + $tx
      $y = [double]$p[1] * $s + $ty
      if ($first) { [void]$sb.Append("M$(Fmt $x) $(Fmt $y)"); $first = $false }
      else { [void]$sb.Append("L$(Fmt $x) $(Fmt $y)") }
    }
    [void]$sb.Append('Z')
    $out += $sb.ToString()
  }
  $out
}

# logo.svg : 144x144 box, R height 100 centered
$ls = 100.0 / $mh
$lw = $mw * $ls
$ltx = (144 - $lw) / 2; $lty = (144 - 100) / 2
$rlogoPaths = Build-RPaths $json.loops $ls $ltx $lty
$logoBody = ($rlogoPaths | ForEach-Object { "<path d=`"$_`" fill=`"white`"/>" }) -join "`n"
$logoSvg = @"
<svg preserveAspectRatio="none" width="100%" height="100%" overflow="visible" style="display: block;" viewBox="0 0 144 144" fill="none" xmlns="http://www.w3.org/2000/svg">
$logoBody
</svg>
"@
[System.IO.File]::WriteAllText("$Base\assets\logo.svg", $logoSvg, (New-Object System.Text.UTF8Encoding($false)))

# footer-logo.svg : 25x25 box, R height 17.5 centered
$fs = 17.5 / $mh
$fw = $mw * $fs
$ftx = (25 - $fw) / 2; $fty = (25 - 17.5) / 2
$rfootPaths = Build-RPaths $json.loops $fs $ftx $fty
$footBody = ($rfootPaths | ForEach-Object { "<path d=`"$_`" fill=`"white`"/>" }) -join "`n"
$footSvg = @"
<svg preserveAspectRatio="none" width="100%" height="100%" overflow="visible" style="display: block;" viewBox="0 0 25 25" fill="none" xmlns="http://www.w3.org/2000/svg">
$footBody
</svg>
"@
[System.IO.File]::WriteAllText("$Base\assets\footer-logo.svg", $footSvg, (New-Object System.Text.UTF8Encoding($false)))

# ============ 2. REOCLIENT wordmark from Outfit-Bold ============
$fontPath = "$Base\work\assets\polyplus\mainmenu\font\Outfit-Bold.ttf"
$uri = New-Object System.Uri("file:///$([System.IO.Path]::GetDirectoryName($fontPath).Replace('\','/'))/")
$ff = New-Object System.Windows.Media.FontFamily($uri, "./#Outfit")
$tf = New-Object System.Windows.Media.Typeface($ff, [System.Windows.FontStyles]::Normal, [System.Windows.FontWeights]::Bold, [System.Windows.FontStretches]::Normal)

function Make-Ft([string]$t) {
  New-Object System.Windows.Media.FormattedText($t, [System.Globalization.CultureInfo]::InvariantCulture, [System.Windows.FlowDirection]::LeftToRight, $tf, 100.0, [System.Windows.Media.Brushes]::White, 1.0)
}
$ftReo = Make-Ft "REO"; $ftClient = Make-Ft "CLIENT"; $ftReoC = Make-Ft "REOC"; $ftC = Make-Ft "C"
$x0 = $ftReoC.Width - $ftC.Width
$geoReo = $ftReo.BuildGeometry((New-Object System.Windows.Point(0, 0)))
$geoClient = $ftClient.BuildGeometry((New-Object System.Windows.Point($x0, 0)))

$b1 = $geoReo.Bounds; $b2 = $geoClient.Bounds
$minX = [Math]::Min($b1.X, $b2.X); $minY = [Math]::Min($b1.Y, $b2.Y)
$maxX = [Math]::Max($b1.X + $b1.Width, $b2.X + $b2.Width); $maxY = [Math]::Max($b1.Y + $b1.Height, $b2.Y + $b2.Height)
$s = 19.0 / ($maxY - $minY)
$tx = -$minX * $s; $ty = -$minY * $s
$wmW = ($maxX - $minX) * $s

function Emit-Geometry([System.Windows.Media.Geometry]$geo) {
  if ($geo -is [System.Windows.Media.GeometryGroup]) {
    $out = ''
    foreach ($c in $geo.Children) { $out += Emit-Geometry $c }
    return $out
  }
  if ($geo -is [System.Windows.Media.PathGeometry]) { $pg = $geo }
  else { $pg = $geo.GetFlattenedPathGeometry(0.01, [System.Windows.Media.ToleranceType]::Absolute) }

  $sb = New-Object System.Text.StringBuilder
  $X = { param($v) Fmt ($v * $s + $tx) }
  $Y = { param($v) Fmt ($v * $s + $ty) }
  foreach ($fig in $pg.Figures) {
    $curX = $fig.StartPoint.X; $curY = $fig.StartPoint.Y
    [void]$sb.Append("M$(& $X $curX) $(& $Y $curY)")
    foreach ($seg in $fig.Segments) {
      switch ($seg.GetType().Name) {
        'LineSegment' {
          $curX = $seg.Point.X; $curY = $seg.Point.Y
          [void]$sb.Append("L$(& $X $curX) $(& $Y $curY)")
        }
        'PolyLineSegment' {
          foreach ($p in $seg.Points) { $curX = $p.X; $curY = $p.Y; [void]$sb.Append("L$(& $X $curX) $(& $Y $curY)") }
        }
        'BezierSegment' {
          $p1 = $seg.Point1; $p2 = $seg.Point2; $p3 = $seg.Point3
          [void]$sb.Append("C$(& $X $p1.X) $(& $Y $p1.Y) $(& $X $p2.X) $(& $Y $p2.Y) $(& $X $p3.X) $(& $Y $p3.Y)")
          $curX = $p3.X; $curY = $p3.Y
        }
        'PolyBezierSegment' {
          $pts = $seg.Points
          for ($i = 0; $i + 2 -lt $pts.Count; $i += 3) {
            $p1 = $pts[$i]; $p2 = $pts[$i+1]; $p3 = $pts[$i+2]
            [void]$sb.Append("C$(& $X $p1.X) $(& $Y $p1.Y) $(& $X $p2.X) $(& $Y $p2.Y) $(& $X $p3.X) $(& $Y $p3.Y)")
            $curX = $p3.X; $curY = $p3.Y
          }
        }
        'QuadraticBezierSegment' {
          $p1 = $seg.Point1; $p2 = $seg.Point2
          $c1x = $curX + 2.0/3.0 * ($p1.X - $curX); $c1y = $curY + 2.0/3.0 * ($p1.Y - $curY)
          $c2x = $p2.X + 2.0/3.0 * ($p1.X - $p2.X); $c2y = $p2.Y + 2.0/3.0 * ($p1.Y - $p2.Y)
          [void]$sb.Append("C$(& $X $c1x) $(& $Y $c1y) $(& $X $c2x) $(& $Y $c2y) $(& $X $p2.X) $(& $Y $p2.Y)")
          $curX = $p2.X; $curY = $p2.Y
        }
        'PolyQuadraticBezierSegment' {
          $pts = $seg.Points
          for ($i = 0; $i + 1 -lt $pts.Count; $i += 2) {
            $p1 = $pts[$i]; $p2 = $pts[$i+1]
            $c1x = $curX + 2.0/3.0 * ($p1.X - $curX); $c1y = $curY + 2.0/3.0 * ($p1.Y - $curY)
            $c2x = $p2.X + 2.0/3.0 * ($p1.X - $p2.X); $c2y = $p2.Y + 2.0/3.0 * ($p1.Y - $p2.Y)
            [void]$sb.Append("C$(& $X $c1x) $(& $Y $c1y) $(& $X $c2x) $(& $Y $c2y) $(& $X $p2.X) $(& $Y $p2.Y)")
            $curX = $p2.X; $curY = $p2.Y
          }
        }
        default {
          # arc or unknown: flatten the whole geometry instead
          $flat = $pg.GetFlattenedPathGeometry(0.01, [System.Windows.Media.ToleranceType]::Absolute)
          return Emit-Geometry $flat
        }
      }
    }
    if ($fig.IsClosed) { [void]$sb.Append('Z') }
  }
  $sb.ToString()
}

$reoPath = Emit-Geometry $geoReo
$clientPath = Emit-Geometry $geoClient
$wmSvg = @"
<svg width="$(Fmt $wmW)" height="19" viewBox="0 0 $(Fmt $wmW) 19" fill="none" xmlns="http://www.w3.org/2000/svg">
<path d="$reoPath" fill="#5187F1"/>
<path d="$clientPath" fill="#1878F1"/>
</svg>
"@
[System.IO.File]::WriteAllText("$Base\assets\oneclient.svg", $wmSvg, (New-Object System.Text.UTF8Encoding($false)))

"logo.svg:        R ${lw}x100 at ($([Math]::Round($ltx,1)),$([Math]::Round($lty,1)))  paths: $($rlogoPaths.Count)"
"footer-logo.svg: R ${fw}x17.5 at ($([Math]::Round($ftx,2)),$([Math]::Round($fty,2)))  paths: $($rfootPaths.Count)"
"oneclient.svg:   REOCLIENT $(Fmt $wmW)x19  reo chars: $($reoPath.Length)  client chars: $($clientPath.Length)"

# command letters used
$used = @{}
foreach ($p in @($reoPath, $clientPath, $rlogoPaths + $rfootPaths)) {
  foreach ($ch in [regex]::Matches($p, '[A-Za-z]')) { $used[$ch.Value] = $true }
}
"commands in generated svgs: " + (($used.Keys | Sort-Object) -join ' ')

# ============ 3. mock preview ============
function New-RoundRect($x, $y, $w, $h, $r) {
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $p.AddArc($x, $y, 2*$r, 2*$r, 180, 90)
  $p.AddArc($x+$w-2*$r, $y, 2*$r, 2*$r, 270, 90)
  $p.AddArc($x+$w-2*$r, $y+$h-2*$r, 2*$r, 2*$r, 0, 90)
  $p.AddArc($x, $y+$h-2*$r, 2*$r, 2*$r, 90, 90)
  $p.CloseFigure()
  return $p
}

$W = 1280; $Hc = 700
$bmp = New-Object System.Drawing.Bitmap($W, $Hc)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$g.Clear([System.Drawing.Color]::FromArgb(255, 8, 12, 20))
$panelBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 13, 19, 31))
$g.FillRectangle($panelBrush, 40, 40, 1200, 620)
$panelBrush.Dispose()

$tracePath = New-Object System.Drawing.Drawing2D.GraphicsPath([System.Drawing.Drawing2D.FillMode]::Alternate)
$sR = 260.0 / $mh
$txR = 300 - ($mw * $sR) / 2; $tyR = 120
foreach ($l in $json.loops) {
  $pts = New-Object System.Collections.Generic.List[System.Drawing.PointF]
  foreach ($p in $l) { $pts.Add([System.Drawing.PointF]::new([float]($txR + [double]$p[0]*$sR), [float]($tyR + [double]$p[1]*$sR))) }
  $tracePath.AddPolygon($pts.ToArray())
}
$white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
$g.FillPath($white, $tracePath)

$pfc = New-Object System.Drawing.Text.PrivateFontCollection
$pfc.AddFontFile($fontPath)
$fam = $pfc.Families[0]
$menuFont = New-Object System.Drawing.Font($fam, 44, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$g.DrawString("REOCLIENT", $menuFont, $white, [System.Drawing.RectangleF]::new(80, 420, 440, 70), $sf)

$fsR = 22.0 / $mh
$ftxR = 80; $ftyR = 540
$footPath = New-Object System.Drawing.Drawing2D.GraphicsPath([System.Drawing.Drawing2D.FillMode]::Alternate)
foreach ($l in $json.loops) {
  $pts = New-Object System.Collections.Generic.List[System.Drawing.PointF]
  foreach ($p in $l) { $pts.Add([System.Drawing.PointF]::new([float]($ftxR + [double]$p[0]*$fsR), [float]($ftyR + [double]$p[1]*$fsR))) }
  $footPath.AddPolygon($pts.ToArray())
}
$g.FillPath($white, $footPath)
$footFont = New-Object System.Drawing.Font($fam, 15, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$g.DrawString("REOCLIENT", $footFont, $white, [float]($ftxR + $mw*$fsR + 8), [float]($ftyR + 2))

$card = New-RoundRect 700 300 520 220 14
$cardBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 17, 24, 39))
$g.FillPath($cardBrush, $card)
$reoFont = New-Object System.Drawing.Font($fam, 26, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$blue1 = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 81, 135, 241))
$blue2 = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 24, 120, 241))
$wReo = $g.MeasureString("REO", $reoFont)
$baseX = 720; $baseY = 330
$g.DrawString("REO", $reoFont, $blue1, [float]$baseX, [float]$baseY)
$g.DrawString("CLIENT", $reoFont, $blue2, [float]($baseX + $wReo.Width - 2), [float]$baseY)
$capFont = New-Object System.Drawing.Font("Segoe UI", 13, [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
$gray = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 148, 163, 184))
$g.DrawString("settings header wordmark (generated SVG)", $capFont, $gray, 720, 400)
$g.DrawString("main menu lockup + footer mark (traced R)", $capFont, $gray, 80, 620)

$bmp.Save("$Base\assets\preview_mock.png", [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
"mock preview saved"