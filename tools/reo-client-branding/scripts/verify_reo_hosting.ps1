# Verify a built ReoHosting tree against the source bundles and the task manifest.
#
# Checks:
#   * catalog metadata.json has no BOM, parses, every path exists and sha1 matches
#   * every rebuilt bundle has the same zip entry names as the original
#   * the index has no BOM and the same bundle-level fields
#   * replaced entries are external (no cdn.modrinth.com URL), point at
#     <ModBaseUrl>/<outName>, carry the built jar's sha1/sha512/size and path
#   * unreplaced entries serialize identically to the original
#   * no cdn.modrinth.com Polyfrost entry is left anywhere
param(
  [Parameter(Mandatory = $true)][string]$ModBaseUrl,
  [string]$HostingDir = "C:\Users\drako\Documents\Default Project\ReoHosting",
  [string]$JarDir = "",
  [string]$TasksPath = "C:\Users\drako\Documents\Default Project\ReoLauncher\tools\reo-client-branding\manifests\reomods-all-versions.json",
  [string]$BundleCacheDir = (Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\caches\bundles\bundles'),
  [string]$OrgProjectsPath = "C:\Users\drako\AppData\Local\Temp\opencode\polyfrost_org_projects.json"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$ModBaseUrl = $ModBaseUrl.TrimEnd('/')
if (-not $JarDir) { $JarDir = Join-Path $HostingDir 'mods' }
$problems = New-Object System.Collections.ArrayList

function Add-Problem([string]$msg) { [void]$problems.Add($msg) }
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
function Test-NoBom([byte[]]$bytes, [string]$label) {
  if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { Add-Problem "BOM present in $label" }
}

$tasks = Get-Content -LiteralPath $TasksPath -Raw | ConvertFrom-Json
$byUrl = @{}
foreach ($t in $tasks) {
  $jar = Join-Path $JarDir $t.outName
  if (-not (Test-Path -LiteralPath $jar)) { Add-Problem "missing built jar $($t.outName)"; continue }
  $byUrl[$t.url] = [pscustomobject]@{
    outName = $t.outName
    sha1    = (Get-FileHash -Algorithm SHA1 -LiteralPath $jar).Hash.ToLower()
    sha512  = (Get-FileHash -Algorithm SHA512 -LiteralPath $jar).Hash.ToLower()
    bytes   = (Get-Item -LiteralPath $jar).Length
    url     = "$ModBaseUrl/$([System.Uri]::EscapeDataString($t.outName))"
  }
}
"tasks: $($tasks.Count)  jars mapped: $($byUrl.Count)"

$org = Get-Content -LiteralPath $OrgProjectsPath -Raw | ConvertFrom-Json
$orgIds = @{}
foreach ($p in $org) { $orgIds[$p.id] = $true }

# --- catalog ---
$catalogPath = Join-Path $HostingDir 'reo\bundles\metadata.json'
$catalogBytes = [System.IO.File]::ReadAllBytes($catalogPath)
Test-NoBom $catalogBytes 'metadata.json'
$catalog = [System.Text.Encoding]::UTF8.GetString($catalogBytes) | ConvertFrom-Json
$catalogEntries = 0
foreach ($ver in $catalog.versions.PSObject.Properties) {
  foreach ($loader in $ver.Value.PSObject.Properties) {
    foreach ($entry in @($loader.Value)) {
      $catalogEntries++
      $rel = $entry.path.TrimStart('/')
      $file = Join-Path $HostingDir $rel
      if (-not (Test-Path -LiteralPath $file)) { Add-Problem "catalog path missing on disk: $($entry.path)"; continue }
      $sha1 = (Get-FileHash -Algorithm SHA1 -LiteralPath $file).Hash.ToLower()
      if ($sha1 -ne $entry.sha1) { Add-Problem "catalog sha1 mismatch for $($entry.path)" }
    }
  }
}
"catalog entries: $catalogEntries  (expected 31)"

# --- bundles ---
$bundleNames = @(Get-ChildItem (Join-Path $HostingDir 'reo\bundles\generated') -Filter *.mrpack | ForEach-Object { $_.Name })
$totalReplaced = 0
$totalUntouched = 0
foreach ($name in $bundleNames) {
  $newPath = Join-Path $HostingDir "reo\bundles\generated\$name"
  $srcPath = Join-Path $BundleCacheDir $name
  if (-not (Test-Path -LiteralPath $srcPath)) { Add-Problem "original bundle missing: $name"; continue }

  # zip entry names
  $zNew = [System.IO.Compression.ZipFile]::OpenRead($newPath)
  $newNames = @($zNew.Entries | ForEach-Object { $_.FullName }); $zNew.Dispose()
  $zSrc = [System.IO.Compression.ZipFile]::OpenRead($srcPath)
  $srcNames = @($zSrc.Entries | ForEach-Object { $_.FullName }); $zSrc.Dispose()
  if (($newNames -join "`n") -ne ($srcNames -join "`n")) { Add-Problem "zip entry names differ: $name" }

  $newIdxBytes = Read-ZipEntryBytes $newPath 'modrinth.index.json'
  Test-NoBom $newIdxBytes "$name :: modrinth.index.json"
  $srcIdxBytes = Read-ZipEntryBytes $srcPath 'modrinth.index.json'
  $newIdx = [System.Text.Encoding]::UTF8.GetString($newIdxBytes) | ConvertFrom-Json
  $srcIdx = [System.Text.Encoding]::UTF8.GetString($srcIdxBytes) | ConvertFrom-Json

  foreach ($fld in @('name', 'versionId', 'category', 'enabled')) {
    $a = $srcIdx.$fld | ConvertTo-Json -Compress
    $b = $newIdx.$fld | ConvertTo-Json -Compress
    if ($a -ne $b) { Add-Problem "$name :: bundle field '$fld' changed ($a -> $b)" }
  }
  $depA = $srcIdx.dependencies | ConvertTo-Json -Depth 10 -Compress
  $depB = $newIdx.dependencies | ConvertTo-Json -Depth 10 -Compress
  if ($depA -ne $depB) { Add-Problem "$name :: dependencies changed" }

  $newFiles = @($newIdx.files)
  $srcFiles = @($srcIdx.files)
  if ($newFiles.Count -ne $srcFiles.Count) { Add-Problem "$name :: file count changed"; continue }

  for ($i = 0; $i -lt $srcFiles.Count; $i++) {
    $sf = $srcFiles[$i]
    $nf = $newFiles[$i]
    $origUrl = [string]$sf.downloads[0]
    if ($byUrl.ContainsKey($origUrl)) {
      $j = $byUrl[$origUrl]
      $totalReplaced++
      if ($nf.downloads.Count -ne 1) { Add-Problem "$name :: ${origUrl}: downloads count $($nf.downloads.Count)"; continue }
      if ($nf.downloads[0] -ne $j.url) { Add-Problem "$name :: ${origUrl}: download url '$($nf.downloads[0])' != '$($j.url)'" }
      if ($nf.path -ne "mods/$($j.outName)") { Add-Problem "$name :: ${origUrl}: path '$($nf.path)'" }
      if (([string]$nf.hashes.sha1).ToLower() -ne $j.sha1) { Add-Problem "$name :: ${origUrl}: sha1 mismatch" }
      if (([string]$nf.hashes.sha512).ToLower() -ne $j.sha512) { Add-Problem "$name :: ${origUrl}: sha512 mismatch" }
      if ([int64]$nf.fileSize -ne [int64]$j.bytes) { Add-Problem "$name :: ${origUrl}: fileSize mismatch" }
      if (-not $nf.overrides -or -not $nf.overrides.name) { Add-Problem "$name :: ${origUrl}: missing overrides.name" }
      if ($sf.id -ne $nf.id) { Add-Problem "$name :: ${origUrl}: id changed" }
      if ($sf.enabled -ne $nf.enabled -or $sf.hidden -ne $nf.hidden) { Add-Problem "$name :: ${origUrl}: enabled/hidden changed" }
    } else {
      $totalUntouched++
      $a = $sf | ConvertTo-Json -Depth 100 -Compress
      $b = $nf | ConvertTo-Json -Depth 100 -Compress
      if ($a -ne $b) { Add-Problem "$name :: untouched entry differs at index $i ($($sf.path))" }
    }
  }

  # no leftover polyfrost managed entries
  foreach ($nf in $newFiles) {
    foreach ($u in @($nf.downloads)) {
      $m = [regex]::Match([string]$u, '^https://cdn\.modrinth\.com/data/([^/]+)/')
      if ($m.Success -and $orgIds.ContainsKey($m.Groups[1].Value)) {
        Add-Problem "$name :: leftover polyfrost managed entry $($nf.path) -> $u"
      }
    }
  }
}

$report = Get-Content (Join-Path $HostingDir 'bundle-report.json') -Raw | ConvertFrom-Json
"bundles checked: $($bundleNames.Count)"
"entries replaced: $totalReplaced  (report says $(($report | Measure-Object replaced -Sum).Sum))"
"entries untouched: $totalUntouched"
if ($problems.Count -gt 0) {
  "PROBLEMS: $($problems.Count)"
  $problems | Select-Object -First 40 | ForEach-Object { "  - $_" }
  exit 1
}
"ALL CHECKS PASSED"
