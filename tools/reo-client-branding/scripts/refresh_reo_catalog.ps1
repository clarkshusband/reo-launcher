# Refresh a Reo bundle catalog in place.
#
# For every bundle listed in the catalog:
#   * rewrite modrinth.index.json entries whose file name matches a built jar
#     (hashes.sha1/sha512/fileSize) so the bundle references the current jars
#   * rewrite the bundle zip
#   * update the catalog entry path to /reo/bundles/generated/<name> and its sha1
#
# usage:
#   powershell -ExecutionPolicy Bypass -File refresh_reo_catalog.ps1 `
#     -BundlesDir <repo>/reo/bundles/generated `
#     -JarDir <repo>/ReoMods `
#     -CatalogPath <repo>/reo/bundles/metadata.json
param(
  [Parameter(Mandatory = $true)][string]$BundlesDir,
  [Parameter(Mandatory = $true)][string]$JarDir,
  [Parameter(Mandatory = $true)][string]$CatalogPath
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$jarCache = @{}
$done = @{}

function Get-JarFacts([string]$leaf) {
  if ($script:jarCache.ContainsKey($leaf)) { return $script:jarCache[$leaf] }
  $jar = Join-Path $JarDir $leaf
  if (-not (Test-Path -LiteralPath $jar)) { return $null }
  $facts = [pscustomobject]@{
    sha1   = (Get-FileHash -Algorithm SHA1 -LiteralPath $jar).Hash.ToLower()
    sha512 = (Get-FileHash -Algorithm SHA512 -LiteralPath $jar).Hash.ToLower()
    bytes  = [int64](Get-Item -LiteralPath $jar).Length
  }
  $script:jarCache[$leaf] = $facts
  return $facts
}

function Update-Bundle([string]$name) {
  if ($script:done.ContainsKey($name)) { return $script:done[$name] }
  $bundlePath = Join-Path $BundlesDir $name
  if (-not (Test-Path -LiteralPath $bundlePath)) { throw "missing bundle: $bundlePath" }

  $zip = [System.IO.Compression.ZipFile]::OpenRead($bundlePath)
  $entries = @()
  $indexText = $null
  foreach ($e in $zip.Entries) {
    $ms = New-Object IO.MemoryStream
    $s = $e.Open(); $s.CopyTo($ms); $s.Close()
    if ($e.FullName -eq 'modrinth.index.json') {
      $indexText = [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
    } else {
      $entries += [pscustomobject]@{ Name = $e.FullName; Bytes = $ms.ToArray(); Time = $e.LastWriteTime }
    }
  }
  $zip.Dispose()

  $index = $indexText | ConvertFrom-Json
  $updated = 0
  $changed = 0
  foreach ($f in @($index.files)) {
    if (-not $f.path) { continue }
    $leaf = Split-Path ([string]$f.path) -Leaf
    $facts = Get-JarFacts $leaf
    if (-not $facts) { continue }
    if ([string]$f.hashes.sha1 -ne $facts.sha1 -or
        [string]$f.hashes.sha512 -ne $facts.sha512 -or
        [int64]$f.fileSize -ne $facts.bytes) {
      $changed++
    }
    $f.hashes.sha1 = $facts.sha1
    $f.hashes.sha512 = $facts.sha512
    $f.fileSize = $facts.bytes
    $updated++
  }

  # Leave untouched bundles byte-identical; only rebuild when a referenced jar
  # actually changed.  This keeps the launcher from re-downloading everything.
  if ($changed -eq 0) {
    $sha1 = (Get-FileHash -Algorithm SHA1 -LiteralPath $bundlePath).Hash.ToLower()
    $result = [pscustomobject]@{ Name = $name; Sha1 = $sha1; Updated = $updated; Changed = 0 }
    $script:done[$name] = $result
    return $result
  }

  $newIndex = $utf8NoBom.GetBytes(($index | ConvertTo-Json -Depth 100 -Compress))
  $tmp = "$bundlePath.tmp"
  Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
  $archive = [System.IO.Compression.ZipFile]::Open($tmp, [System.IO.Compression.ZipArchiveMode]::Create)
  foreach ($en in $entries) {
    $ne = $archive.CreateEntry($en.Name, [System.IO.Compression.CompressionLevel]::Optimal)
    try { $ne.LastWriteTime = $en.Time } catch { }
    $st = $ne.Open(); $st.Write($en.Bytes, 0, $en.Bytes.Length); $st.Close()
  }
  $ne = $archive.CreateEntry('modrinth.index.json', [System.IO.Compression.CompressionLevel]::Optimal)
  $st = $ne.Open(); $st.Write($newIndex, 0, $newIndex.Length); $st.Close()
  $archive.Dispose()
  Move-Item -LiteralPath $tmp -Destination $bundlePath -Force

  $sha1 = (Get-FileHash -Algorithm SHA1 -LiteralPath $bundlePath).Hash.ToLower()
  $result = [pscustomobject]@{ Name = $name; Sha1 = $sha1; Updated = $updated; Changed = $changed }
  $script:done[$name] = $result
  return $result
}

$catalog = Get-Content -LiteralPath $CatalogPath -Raw | ConvertFrom-Json
$total = 0
foreach ($ver in $catalog.versions.PSObject.Properties) {
  foreach ($loader in $ver.Value.PSObject.Properties) {
    foreach ($entry in @($loader.Value)) {
      $name = Split-Path ([string]$entry.path) -Leaf
      $r = Update-Bundle $name
      $entry.path = "/reo/bundles/generated/$name"
      $entry.sha1 = $r.Sha1
      $total++
      Write-Host ("  {0,-34} jars={1,-3} sha1={2}" -f $name, $r.Updated, $r.Sha1)
    }
  }
}

$catalogText = $catalog | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($CatalogPath, $catalogText, $utf8NoBom)
Write-Host "catalog entries: $total"
