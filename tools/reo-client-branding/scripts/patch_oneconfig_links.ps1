# Replaces the OneConfig GitHub repository link with the ReoMC Discord in the
# bundled OneConfig / Compose Multiplatform metadata.
#
#   https://github.com/Polyfrost/OneConfig  ->  https://discord.gg/reomc
#
# Touched entries:
#   OneConfig-*.jar : fabric.mod.json
#                     META-INF/jars/26.3-fabric-*.jar!fabric.mod.json
#   compose-bundle-*.jar : fabric.mod.json
#
# usage:
#   powershell -ExecutionPolicy Bypass -File patch_oneconfig_links.ps1
#   powershell -ExecutionPolicy Bypass -File patch_oneconfig_links.ps1 -Swap
param(
  [string]$ModsDir = "C:\Users\drako\AppData\Roaming\drako\ReoLauncher-dev\clusters\26.3 Fabric\mods",
  [string]$WorkDir = "C:\Users\drako\AppData\Local\Temp\opencode\oneconfig_links",
  [switch]$Swap
)

$ErrorActionPreference = 'Stop'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolkit = Split-Path -Parent $scriptDir
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$old = 'https://github.com/Polyfrost/OneConfig'
$new = 'https://discord.gg/reomc'

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$jdkRoot = Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\java'
$javac = Get-ChildItem $jdkRoot -Recurse -Filter javac.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $javac) { throw "javac.exe not found under $jdkRoot" }
$javaBin = $javac.DirectoryName
$java = Join-Path $javaBin 'java.exe'

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

function Replace-InJson([byte[]]$bytes) {
  $text = [System.Text.Encoding]::UTF8.GetString($bytes)
  if (-not $text.Contains($old)) { return $null }
  return $utf8NoBom.GetBytes($text.Replace($old, $new))
}

function Get-EntryCount([string]$jar) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try { return $zip.Entries.Count } finally { $zip.Dispose() }
}

New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$cpToolDir = Join-Path $WorkDir 'cptool'
New-Item -ItemType Directory -Path $cpToolDir -Force | Out-Null
& $javac -d $cpToolDir (Join-Path $scriptDir 'CpTool.java')
if ($LASTEXITCODE -ne 0) { throw "javac CpTool.java failed" }

$backupDir = Join-Path $toolkit 'backup\mods'
New-Item -ItemType Directory -Path $backupDir -Force | Out-Null

$oneConfig = Get-ChildItem $ModsDir -Filter 'OneConfig-*.jar' | Select-Object -First 1
$compose = Get-ChildItem $ModsDir -Filter 'compose-bundle-*.jar' | Select-Object -First 1
if (-not $oneConfig) { throw "OneConfig jar not found" }
if (-not $compose) { throw "compose-bundle jar not found" }

$results = @()

# ---------- OneConfig (outer + nested metadata) ----------
$stem = [System.IO.Path]::GetFileNameWithoutExtension($oneConfig.Name)
$caseDir = Join-Path $WorkDir $stem
New-Item -ItemType Directory -Path $caseDir -Force | Out-Null
$inJar = Join-Path $caseDir 'in.jar'
$outJar = Join-Path $caseDir 'out.jar'
Copy-Item -LiteralPath $oneConfig.FullName -Destination $inJar -Force

# outer metadata
$fmj = Replace-InJson (Get-ZipEntryBytes $inJar 'fabric.mod.json')
if (-not $fmj) { throw "OneConfig outer fabric.mod.json has no OneConfig GitHub link" }
Set-ZipEntryBytes $inJar 'fabric.mod.json' $fmj

# nested jar metadata
$zip = [System.IO.Compression.ZipFile]::OpenRead($inJar)
try {
  $nestedEntry = $zip.Entries | Where-Object { $_.FullName -match '^META-INF/jars/26\.3-fabric-.*\.jar$' } | Select-Object -First 1
  if (-not $nestedEntry) { throw "nested OneConfig jar not found" }
  $nestedName = $nestedEntry.FullName
} finally { $zip.Dispose() }
$nestedJar = Join-Path $caseDir 'oneconfig-nested.jar'
[System.IO.File]::WriteAllBytes($nestedJar, (Get-ZipEntryBytes $inJar $nestedName))
$nestedFmj = Replace-InJson (Get-ZipEntryBytes $nestedJar 'fabric.mod.json')
if (-not $nestedFmj) { throw "nested OneConfig fabric.mod.json has no OneConfig GitHub link" }
Set-ZipEntryBytes $nestedJar 'fabric.mod.json' $nestedFmj
$nestedFinal = Join-Path $caseDir 'oneconfig-nested-final.jar'
& $java -cp $cpToolDir CpTool --normalize $nestedJar $nestedFinal
if ($LASTEXITCODE -ne 0) { throw "normalize nested OneConfig failed" }
Set-ZipEntryBytes $inJar $nestedName ([System.IO.File]::ReadAllBytes($nestedFinal))

& $java -cp $cpToolDir CpTool --normalize $inJar $outJar
if ($LASTEXITCODE -ne 0) { throw "normalize OneConfig failed" }

if ((Get-EntryCount $inJar) -ne (Get-EntryCount $outJar)) { throw "OneConfig entry count changed" }
$outerText = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $outJar 'fabric.mod.json'))
$nestedFinalText = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $nestedFinal 'fabric.mod.json'))
if ($outerText.Contains($old) -or $nestedFinalText.Contains($old)) { throw "OneConfig GitHub link still present" }
if (-not $outerText.Contains($new) -or -not $nestedFinalText.Contains($new)) { throw "Discord link missing" }
& $java -cp $cpToolDir CpTool --scan $outJar ReoMC | Out-Null
if ($LASTEXITCODE -ne 0) { throw "OneConfig jar is not stream-readable" }

$results += [pscustomobject]@{ File = $oneConfig.Name; OutJar = $outJar; Entries = (Get-EntryCount $outJar) }

# ---------- Compose Multiplatform metadata ----------
$stem2 = [System.IO.Path]::GetFileNameWithoutExtension($compose.Name)
$caseDir2 = Join-Path $WorkDir $stem2
New-Item -ItemType Directory -Path $caseDir2 -Force | Out-Null
$inJar2 = Join-Path $caseDir2 'in.jar'
$outJar2 = Join-Path $caseDir2 'out.jar'
Copy-Item -LiteralPath $compose.FullName -Destination $inJar2 -Force
$fmj2 = Replace-InJson (Get-ZipEntryBytes $inJar2 'fabric.mod.json')
if (-not $fmj2) { throw "compose-bundle fabric.mod.json has no OneConfig GitHub link" }
Set-ZipEntryBytes $inJar2 'fabric.mod.json' $fmj2
& $java -cp $cpToolDir CpTool --normalize $inJar2 $outJar2
if ($LASTEXITCODE -ne 0) { throw "normalize compose-bundle failed" }
if ((Get-EntryCount $inJar2) -ne (Get-EntryCount $outJar2)) { throw "compose-bundle entry count changed" }
$text2 = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $outJar2 'fabric.mod.json'))
if ($text2.Contains($old) -or -not $text2.Contains($new)) { throw "compose-bundle link not replaced" }
& $java -cp $cpToolDir CpTool --scan $outJar2 ReoMC | Out-Null
if ($LASTEXITCODE -ne 0) { throw "compose-bundle jar is not stream-readable" }

$results += [pscustomobject]@{ File = $compose.Name; OutJar = $outJar2; Entries = (Get-EntryCount $outJar2) }

$results | Format-Table -AutoSize | Out-String -Width 160

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
