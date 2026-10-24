# Rebuild the Polyfrost bundle catalog with ReoMods jar entries.
#
# For every cached .mrpack bundle:
#   * mod file entries whose original download URL belongs to a ReoMods task are
#     rewritten into self-hosted *external* entries: the cdn.modrinth.com URL is
#     dropped, downloads[0] points at <ModBaseUrl>/<outName>, hashes/fileSize
#     come from the built jar and the path becomes mods/<outName>
#     (a bundle entry is only treated as a Modrinth-managed package when a
#     downloads entry is a cdn.modrinth.com URL, and managed entries are
#     downloaded from Modrinth by version id - never from us)
#   * display metadata for those entries (name/description/authors) is filled
#     into the entry `overrides` so the mod list does not fall back to filenames
#   * every other entry, override file and bundle field is copied unchanged
#   * the bundle's sha1/sha512/size are recomputed
# The catalog metadata.json is copied with the new bundle sha1s and written
# without a BOM (serde_json rejects one).
#
# usage:
#   powershell -ExecutionPolicy Bypass -File build_reo_bundles.ps1 `
#     -ModBaseUrl https://github.com/OWNER/REPO/releases/download/reomods
param(
  [Parameter(Mandatory = $true)][string]$ModBaseUrl,
  [string]$TasksPath = "",
  [string]$BundlesMetaPath = "",
  [string]$JarDir = "C:\Users\drako\Documents\Default Project\ReoHosting\mods",
  [string]$BundleCacheDir = (Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\caches\bundles\bundles'),
  [string]$CatalogPath = (Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\caches\bundles\metadata.json'),
  [string]$OutDir = "C:\Users\drako\Documents\Default Project\ReoHosting"
)

$ErrorActionPreference = 'Stop'
$manifestDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'manifests'
if (-not $TasksPath) { $TasksPath = Join-Path $manifestDir 'reomods-all-versions.json' }
if (-not $BundlesMetaPath) { $BundlesMetaPath = Join-Path $manifestDir 'reomods-all-versions.bundles.json' }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$ModBaseUrl = $ModBaseUrl.TrimEnd('/')

$tasks = Get-Content -LiteralPath $TasksPath -Raw | ConvertFrom-Json
$bundleMeta = Get-Content -LiteralPath $BundlesMetaPath -Raw | ConvertFrom-Json
$outBundles = Join-Path $OutDir 'reo\bundles\generated'
$outCatalog = Join-Path $OutDir 'reo\bundles\metadata.json'
New-Item -ItemType Directory -Force -Path $outBundles, (Split-Path $outCatalog -Parent) | Out-Null

function Read-ZipEntryBytes([string]$jar, [string]$name) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $e = $zip.GetEntry($name)
    if (-not $e) { throw "missing $name in $jar" }
    $ms = New-Object System.IO.MemoryStream
    $s = $e.Open(); $s.CopyTo($ms); $s.Close()
    return $ms.ToArray()
  } finally { $zip.Dispose() }
}

function Try-Read-ZipEntryBytes([string]$jar, [string]$name) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $e = $zip.GetEntry($name)
    if (-not $e) { return $null }
    $ms = New-Object System.IO.MemoryStream
    $s = $e.Open(); $s.CopyTo($ms); $s.Close()
    return $ms.ToArray()
  } finally { $zip.Dispose() }
}

# Map original download URL and used file name -> built jar facts.
$byUrl = @{}
$byName = @{}
$missing = @()
foreach ($t in $tasks) {
  $jar = Join-Path $JarDir $t.outName
  if (-not (Test-Path -LiteralPath $jar)) { $missing += $t.outName; continue }
  $uiName = $null
  $uiDescription = $null
  $fmjBytes = Try-Read-ZipEntryBytes $jar 'fabric.mod.json'
  if ($fmjBytes) {
    $fmj = [System.Text.Encoding]::UTF8.GetString($fmjBytes) | ConvertFrom-Json
    if ($fmj.PSObject.Properties['name']) { $uiName = [string]$fmj.name }
    if ($fmj.PSObject.Properties['description']) { $uiDescription = [string]$fmj.description }
  }
  $facts = [pscustomobject]@{
    outName      = $t.outName
    sha1         = (Get-FileHash -Algorithm SHA1 -LiteralPath $jar).Hash.ToLower()
    sha512       = (Get-FileHash -Algorithm SHA512 -LiteralPath $jar).Hash.ToLower()
    bytes        = (Get-Item -LiteralPath $jar).Length
    url          = "$ModBaseUrl/$([System.Uri]::EscapeDataString($t.outName))"
    uiName       = $uiName
    uiDescription = $uiDescription
  }
  $byUrl[$t.url] = $facts
  $byName[$t.usedName] = $facts
}
if ($missing.Count -gt 0) { throw "missing built jars ($($missing.Count)): $($missing -join ', ')" }

$report = @()
foreach ($b in $bundleMeta) {
  $srcPath = Join-Path $BundleCacheDir $b.name
  if (-not (Test-Path -LiteralPath $srcPath)) { throw "missing bundle: $srcPath" }
  $indexBytes = Read-ZipEntryBytes $srcPath 'modrinth.index.json'
  $index = [System.Text.Encoding]::UTF8.GetString($indexBytes) | ConvertFrom-Json
  $replaced = 0
  foreach ($f in @($index.files)) {
    if (-not $f.downloads -or @($f.downloads).Count -eq 0) { continue }
    $origUrl = [string]$f.downloads[0]
    $j = $null
    if ($byUrl.ContainsKey($origUrl)) {
      $j = $byUrl[$origUrl]
    } else {
      $leaf = Split-Path ([string]$f.path) -Leaf
      $leaf = [System.Uri]::UnescapeDataString($leaf)
      if ($byName.ContainsKey($leaf)) { $j = $byName[$leaf] }
    }
    if (-not $j) { continue }
    $f.path = "mods/$($j.outName)"
    # Only self-hosted URLs remain so the entry parses as an external file;
    # a cdn.modrinth.com URL would make it a managed package and the launcher
    # would download the pristine Polyfrost jar from Modrinth instead.
    $f.downloads = @($j.url)
    $f.hashes.sha1 = $j.sha1
    $f.hashes.sha512 = $j.sha512
    $f.fileSize = [int64]$j.bytes
    $override = [ordered]@{ authors = @('drako') }
    if ($j.uiName) { $override['name'] = $j.uiName }
    if ($j.uiDescription) { $override['description'] = $j.uiDescription }
    $f.overrides = [pscustomobject]$override
    $replaced++
  }
  $newIndexText = $index | ConvertTo-Json -Depth 100 -Compress
  $newIndexBytes = $utf8NoBom.GetBytes($newIndexText)

  $destPath = Join-Path $outBundles $b.name
  $srcZip = [System.IO.Compression.ZipFile]::OpenRead($srcPath)
  try {
    $fs = [System.IO.File]::Open($destPath, [System.IO.FileMode]::Create)
    try {
      $dstZip = New-Object System.IO.Compression.ZipArchive($fs, [System.IO.Compression.ZipArchiveMode]::Create)
      try {
        foreach ($e in $srcZip.Entries) {
          $name = $e.FullName
          $newEntry = $dstZip.CreateEntry($name, [System.IO.Compression.CompressionLevel]::Optimal)
          $os = $newEntry.Open()
          try {
            if ($name -eq 'modrinth.index.json') {
              $os.Write($newIndexBytes, 0, $newIndexBytes.Length)
            } else {
              $s = $e.Open()
              try { $s.CopyTo($os) } finally { $s.Dispose() }
            }
          } finally { $os.Dispose() }
        }
      } finally { $dstZip.Dispose() }
    } finally { $fs.Dispose() }
  } finally { $srcZip.Dispose() }

  $report += [pscustomobject]@{
    name      = $b.name
    oldSha1   = $b.sha1
    newSha1   = (Get-FileHash -Algorithm SHA1 -LiteralPath $destPath).Hash.ToLower()
    newSha512 = (Get-FileHash -Algorithm SHA512 -LiteralPath $destPath).Hash.ToLower()
    bytes     = (Get-Item -LiteralPath $destPath).Length
    replaced  = $replaced
  }
}

# Refresh catalog sha1s and /reo paths.
$catalog = Get-Content -LiteralPath $CatalogPath -Raw | ConvertFrom-Json
$reportByName = @{}
foreach ($r in $report) { $reportByName[$r.name] = $r }
foreach ($ver in $catalog.versions.PSObject.Properties) {
  foreach ($loader in $ver.Value.PSObject.Properties) {
    foreach ($entry in @($loader.Value)) {
      $leaf = Split-Path ([string]$entry.path) -Leaf
      if ($reportByName.ContainsKey($leaf)) {
        $entry.path = "/reo/bundles/generated/$leaf"
        $entry.sha1 = $reportByName[$leaf].newSha1
      }
    }
  }
}
$catalogText = $catalog | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($outCatalog, $catalogText, $utf8NoBom)

$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $OutDir 'bundle-report.json') -Encoding UTF8
""
"=== bundle rebuild ==="
$report | Format-Table -AutoSize | Out-String -Width 160
"bundles: $($report.Count)  entries replaced: $(($report | Measure-Object replaced -Sum).Sum)"
"catalog: $outCatalog"
"jar base: $ModBaseUrl"
