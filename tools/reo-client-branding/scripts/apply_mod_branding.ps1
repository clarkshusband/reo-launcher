# Rebrand the client-integrated (Polyfrost) mods:
#   * PolyBlur / PolyCrosshair / PolyHitbox / PolyNametag / PolySprint / PolyTime /
#     PolyWeather -> ReoBlur / ...  (fabric.mod.json name + in-jar display strings)
#   * remove contact links and Mod Menu links from every bundled Polyfrost mod
#   * description "OneClient" -> "ReoClient"
#
# The client mod list reads fabric.mod.json.  The in-jar display literals (used by
# the OneConfig config screen) are rewritten through the constant pool only, so
# class names, entrypoints, mixin configs and settings file names stay untouched.
#
# usage:
#   powershell -ExecutionPolicy Bypass -File apply_mod_branding.ps1
#   powershell -ExecutionPolicy Bypass -File apply_mod_branding.ps1 -Swap
param(
  [string]$ModsDir = "C:\Users\drako\AppData\Roaming\drako\ReoLauncher-dev\clusters\26.3 Fabric\mods",
  [string]$WorkDir = "C:\Users\drako\AppData\Local\Temp\opencode\mod_branding",
  [switch]$Swap
)

$ErrorActionPreference = 'Stop'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolkit = Split-Path -Parent $scriptDir
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$jdkRoot = Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\java'
$javac = Get-ChildItem $jdkRoot -Recurse -Filter javac.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $javac) { throw "javac.exe not found under $jdkRoot" }
$javaBin = $javac.DirectoryName
$java = Join-Path $javaBin 'java.exe'

# jar file -> new display name ('' = keep name), in-jar constant pool pair ('' = no classes)
$targets = @(
  @{ File = 'polyblur-2.2.7+26.3.jar';            Name = 'ReoBlur';       Pair = 'PolyBlur=>ReoBlur' },
  @{ File = 'polycrosshair-2.0.4+26.3.jar';       Name = 'ReoCrosshair';  Pair = 'PolyCrosshair=>ReoCrosshair' },
  @{ File = 'polyhitbox-1.3.2+26.3.jar';          Name = 'ReoHitbox';     Pair = 'PolyHitbox=>ReoHitbox' },
  @{ File = 'polynametag-1.2.4+26.3.jar';         Name = 'ReoNametag';    Pair = 'PolyNametag=>ReoNametag' },
  @{ File = 'polysprint-1.3.2+26.3.jar';          Name = 'ReoSprint';     Pair = 'PolySprint=>ReoSprint' },
  @{ File = 'polytime-1.3.2+26.3.jar';            Name = 'ReoTime';       Pair = 'PolyTime=>ReoTime' },
  @{ File = 'polyweather-1.3.0+26.3.jar';         Name = 'ReoWeather';    Pair = 'PolyWeather=>ReoWeather' },
  @{ File = 'behindyouv3-3.5.0+26.3.jar';         Name = ''; Pair = '' },
  @{ File = 'chatting-3.2.3+26.3.jar';            Name = ''; Pair = '' },
  @{ File = 'chattweaks-1.2.2+26.3.jar';          Name = ''; Pair = '' },
  @{ File = 'colorsaturation-1.3.0+26.3.jar';     Name = ''; Pair = '' },
  @{ File = 'CrashPatch-2.2.1+26.3.jar';          Name = ''; Pair = '' },
  @{ File = 'damagetint-3.7.2+26.3.jar';          Name = ''; Pair = '' },
  @{ File = 'evergreenhud-3.9.10+26.3.jar';       Name = ''; Pair = '' },
  @{ File = 'overflowanimations-5.0.8+26.3-fabric.jar'; Name = ''; Pair = '' },
  @{ File = 'overflowparticles-1.2.4+26.3.jar';   Name = ''; Pair = '' },
  @{ File = 'REDACTION-2.1.1+26.3.jar';           Name = ''; Pair = '' },
  @{ File = 'smartsearch-1.2.1.jar';              Name = ''; Pair = '' },
  @{ File = 'soundtweaks-1.0.1+26.3.jar';         Name = ''; Pair = '' },
  @{ File = 'vanillahud-3.5.9+26.3.jar';          Name = ''; Pair = '' }
)

function Get-ZipEntryBytes([string]$jar, [string]$name) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $entry = $zip.GetEntry($name)
    if (-not $entry) { throw "entry not found: $name in $jar" }
    $ms = New-Object System.IO.MemoryStream
    $s = $entry.Open(); $s.CopyTo($ms); $s.Close()
    return $ms.ToArray()
  } finally { $zip.Dispose() }
}

function Set-ZipEntryBytes([string]$jar, [string]$name, [byte[]]$bytes) {
  $zip = [System.IO.Compression.ZipFile]::Open($jar, [System.IO.Compression.ZipArchiveMode]::Update)
  try {
    $entry = $zip.GetEntry($name)
    if ($entry) { $entry.Delete() }
    $newEntry = $zip.CreateEntry($name, [System.IO.Compression.CompressionLevel]::Optimal)
    $s = $newEntry.Open()
    $s.Write($bytes, 0, $bytes.Length)
    $s.Dispose()
  } finally { $zip.Dispose() }
}

function Get-JarEntryMap([string]$jar) {
  $map = @{}
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    foreach ($e in $zip.Entries) {
      if ($e.Name -eq '') { continue }
      $sha = [System.Security.Cryptography.SHA1]::Create()
      $s = $e.Open()
      $hash = [BitConverter]::ToString($sha.ComputeHash($s)).Replace('-', '').ToLowerInvariant()
      $s.Dispose(); $sha.Dispose()
      $map[$e.FullName] = $hash
    }
  } finally { $zip.Dispose() }
  return $map
}

function Assert-JarNames([string]$jar) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $bad = @($zip.Entries | Where-Object { $_.FullName.Contains('\') })
    if ($zip.Entries.Count -eq 0) { throw "empty jar: $jar" }
    if ($bad.Count -gt 0) { throw "jar contains backslash entry names: $jar" }
    return $zip.Entries.Count
  } finally { $zip.Dispose() }
}

New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$cpToolDir = Join-Path $WorkDir 'cptool'
New-Item -ItemType Directory -Path $cpToolDir -Force | Out-Null
& $javac -d $cpToolDir (Join-Path $scriptDir 'CpTool.java')
if ($LASTEXITCODE -ne 0) { throw "javac CpTool.java failed" }

$backupDir = Join-Path $toolkit 'backup\mods'
New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
$results = @()

foreach ($t in $targets) {
  $path = Join-Path $ModsDir $t.File
  if (-not (Test-Path -LiteralPath $path)) { Write-Warning "missing, skipped: $($t.File)"; continue }

  $stem = [System.IO.Path]::GetFileNameWithoutExtension($t.File)
  $caseDir = Join-Path $WorkDir $stem
  New-Item -ItemType Directory -Path $caseDir -Force | Out-Null
  $inJar = Join-Path $caseDir 'in.jar'
  $stage = Join-Path $caseDir 'stage.jar'
  $outJar = Join-Path $caseDir 'out.jar'
  Copy-Item -LiteralPath $path -Destination $inJar -Force

  if ($t.Pair) {
    $pairsPath = Join-Path $caseDir 'pairs.txt'
    [System.IO.File]::WriteAllText($pairsPath, "$($t.Pair)`n", $utf8NoBom)
    & $java -cp $cpToolDir CpTool --apply $inJar $stage $pairsPath
    if ($LASTEXITCODE -ne 0) { throw "CpTool failed for $($t.File)" }
  } else {
    Copy-Item -LiteralPath $inJar -Destination $stage -Force
  }

  $fmjIn = Join-Path $caseDir 'fmj-in.json'
  $fmjOut = Join-Path $caseDir 'fmj-out.json'
  [System.IO.File]::WriteAllBytes($fmjIn, (Get-ZipEntryBytes $stage 'fabric.mod.json'))
  & node (Join-Path $scriptDir 'edit_fabric_mod.js') $fmjIn $fmjOut $t.Name
  if ($LASTEXITCODE -ne 0) { throw "node edit failed for $($t.File)" }
  Set-ZipEntryBytes $stage 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($fmjOut))

  & $java -cp $cpToolDir CpTool --normalize $stage $outJar
  if ($LASTEXITCODE -ne 0) { throw "normalize failed for $($t.File)" }

  $inCount = Assert-JarNames $inJar
  $outCount = Assert-JarNames $outJar
  if ($inCount -ne $outCount) { throw "entry count changed for $($t.File): $inCount -> $outCount" }

  $fmj = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $outJar 'fabric.mod.json')) | ConvertFrom-Json
  if ($t.Name -and $fmj.name -ne $t.Name) { throw "name mismatch for $($t.File): $($fmj.name)" }
  if ($fmj.contact) { throw "contact not removed for $($t.File)" }
  if ($fmj.custom -and $fmj.custom.modmenu -and $fmj.custom.modmenu.links) { throw "modmenu links not removed for $($t.File)" }
  if ($t.Pair) {
    $sep = $t.Pair.IndexOf('=>')
    $oldName = $t.Pair.Substring(0, $sep)
    $newName = $t.Pair.Substring($sep + 2)
    $oldHits = @(& $java -cp $cpToolDir CpTool --scan $outJar $oldName 2>&1)
    $newHits = @(& $java -cp $cpToolDir CpTool --scan $outJar $newName 2>&1)
    if ($oldHits.Count -gt 0) { throw "old display name still present in $($t.File): $oldHits" }
    if ($newHits.Count -eq 0) { throw "new display name not found in $($t.File)" }
  }

  # stream readability
  & $java -cp $cpToolDir CpTool --scan $outJar ReoClient | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "jar is not stream-readable: $($t.File)" }

  $before = Get-JarEntryMap $inJar
  $after = Get-JarEntryMap $outJar
  $changed = @($after.Keys | Where-Object { -not $before.ContainsKey($_) -or $before[$_] -ne $after[$_] })

  $results += [pscustomobject]@{
    File = $t.File
    Name = [string]$fmj.name
    Entries = $outCount
    Changed = $changed.Count
    ChangedNames = ($changed -join ', ')
    OutJar = $outJar
  }
}

$results | Format-Table -AutoSize -Wrap | Out-String -Width 200

if ($Swap) {
  foreach ($r in $results) {
    $dest = Join-Path $ModsDir $r.File
    $backup = Join-Path $backupDir $r.File
    if (-not (Test-Path -LiteralPath $backup)) { Copy-Item -LiteralPath $dest -Destination $backup -Force }
    Remove-Item -LiteralPath $dest -Force
    Copy-Item -LiteralPath $r.OutJar -Destination $dest -Force
    "installed: $($r.File)  $((Get-FileHash -Algorithm SHA1 -LiteralPath $dest).Hash.ToLower())"
  }
}
"done"
