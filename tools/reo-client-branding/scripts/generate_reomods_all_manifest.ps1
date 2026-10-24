# Build the all-versions ReoMods task manifest from the launcher's cached
# Polyfrost bundle catalog.
#
# Every bundle under <BundleCacheDir>*.mrpack is scanned; mod file entries whose
# Modrinth project id belongs to the Polyfrost organization become one task per
# distinct download URL (so the same jar used by several bundles is built once).
#
# Outputs (next to each other):
#   reomods-all-versions.json          task array for build_reomods.ps1
#   reomods-all-versions.bundles.json  bundle list + original sha1/paths
#   reomods-all-versions.part-N.json   per-version partition task arrays
#
# usage:
#   powershell -ExecutionPolicy Bypass -File generate_reomods_all_manifest.ps1 `
#     -OrgProjectsPath <polyfrost_org_projects.json>
param(
  [Parameter(Mandatory = $true)][string]$OrgProjectsPath,
  [string]$BundleCacheDir = (Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\caches\bundles\bundles'),
  [string]$OutDir = ""
)

$ErrorActionPreference = 'Stop'
if (-not $OutDir) { $OutDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'manifests' }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Read-ZipText([string]$jar, [string]$entryName) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $e = $zip.GetEntry($entryName)
    if (-not $e) { throw "missing $entryName in $jar" }
    $ms = New-Object System.IO.MemoryStream
    $s = $e.Open(); $s.CopyTo($ms); $s.Close()
    return [System.Text.Encoding]::UTF8.GetString($ms.ToArray())
  } finally { $zip.Dispose() }
}

$org = Get-Content -LiteralPath $OrgProjectsPath -Raw | ConvertFrom-Json
$byId = @{}
foreach ($p in $org) { $byId[$p.id] = $p.slug }

$versionOrder = @('1.8.9', '1.21.1', '1.21.10', '1.21.11', '26.1.2', '26.2', '26.3')
$partitionByVersion = @{
  '1.8.9'   = 1
  '1.21.1'  = 2
  '1.21.10' = 2
  '1.21.11' = 3
  '26.1.2'  = 3
  '26.2'    = 4
  '26.3'    = 4
}

$tasksByUrl = [ordered]@{}
$bundles = @()
foreach ($b in (Get-ChildItem -LiteralPath $BundleCacheDir -Filter *.mrpack | Sort-Object Name)) {
  $indexText = Read-ZipText $b.FullName 'modrinth.index.json'
  $idx = $indexText | ConvertFrom-Json
  $mc = [string]$idx.dependencies.minecraft
  $loader = 'fabric'
  if ($mc -eq '1.8.9') { $loader = 'ornithe' }
  if (-not $partitionByVersion.ContainsKey($mc)) { throw "unexpected minecraft version in $($b.Name): $mc" }
  $partition = $partitionByVersion[$mc]
  $bundles += [pscustomobject]@{
    name       = $b.Name
    path       = "/reo/bundles/generated/$($b.Name)"
    sha1       = (Get-FileHash -Algorithm SHA1 -LiteralPath $b.FullName).Hash.ToLower()
    sha512     = (Get-FileHash -Algorithm SHA512 -LiteralPath $b.FullName).Hash.ToLower()
    bytes      = $b.Length
    minecraft  = $mc
    loader     = $loader
    partition  = $partition
  }

  foreach ($f in @($idx.files)) {
    if (-not $f.downloads -or @($f.downloads).Count -eq 0) { continue }
    $url = [string]$f.downloads[0]
    $m = [regex]::Match($url, '^https://cdn\.modrinth\.com/data/([^/]+)/')
    if (-not $m.Success) { continue }
    $projectId = $m.Groups[1].Value
    if (-not $byId.ContainsKey($projectId)) { continue }
    $slug = [string]$byId[$projectId]
    if (-not $tasksByUrl.Contains($url)) {
      $usedName = Split-Path ([string]$f.path) -Leaf
      $outName = $usedName
      if ($usedName.StartsWith('Poly')) { $outName = 'Reo' + $usedName.Substring(4) }
      elseif ($usedName.StartsWith('poly')) { $outName = 'reo' + $usedName.Substring(4) }

      $task = [ordered]@{
        slug          = $slug
        projectId     = $projectId
        usedName      = $usedName
        url           = $url
        sha1          = [string]$f.hashes.sha1
        sha512        = [string]$f.hashes.sha512
        fileSize      = [int64]$f.fileSize
        outName       = $outName
        name          = $null
        contact       = 'remove'
        versions      = @()
        bundles       = @()
        partition     = 99
      }
      if ($slug -eq 'polyplus') {
        $task.name = 'ReoPlus'
        $task.nestedAuto = $true
        $task.featuredServer = $true
        $task.strictPairs = $false
        $task.pairs = @(
          'PolyPlus (OneClient)=>ReoPlus (ReoClient)',
          'PolyPlus (ReoClient)=>ReoPlus (ReoClient)'
        )
      } elseif ($slug -eq 'oneconfig') {
        $task.contact = 'discord'
        $task.nestedAuto = $true
      } elseif ($slug -eq 'compose-multiplatform') {
        $task.contact = 'discord'
      } elseif ($slug -in @('polyblur', 'polycrosshair', 'polyhitbox', 'polynametag', 'polysprint', 'polytime', 'polyweather', 'polyzoom')) {
        $task.autoPolyName = $true
      }
      $tasksByUrl[$url] = [pscustomobject]$task
    }
    $t = $tasksByUrl[$url]
    if ($t.versions -notcontains $mc) { $t.versions += $mc }
    if ($t.bundles -notcontains $b.Name) { $t.bundles += $b.Name }
  }
}

foreach ($t in $tasksByUrl.Values) {
  $first = $versionOrder | Where-Object { $t.versions -contains $_ } | Select-Object -First 1
  $t.partition = $partitionByVersion[$first]
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$tasks = @($tasksByUrl.Values | Sort-Object slug, usedName)
$tasks | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutDir 'reomods-all-versions.json') -Encoding UTF8
$bundles | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutDir 'reomods-all-versions.bundles.json') -Encoding UTF8

for ($p = 1; $p -le 4; $p++) {
  $part = @($tasks | Where-Object { $_.partition -eq $p })
  $part | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $OutDir "reomods-all-versions.part-$p.json") -Encoding UTF8
}

"### manifest summary ###"
"tasks: $($tasks.Count)   bundles: $($bundles.Count)"
$tasks | Group-Object slug | Sort-Object Name | ForEach-Object {
  "  {0,-24} {1,3} file(s)  {2,10:N0} KB" -f $_.Name, $_.Count, (($_.Group | Measure-Object fileSize -Sum).Sum / 1KB)
}
""
"total download: $([math]::Round((($tasks | Measure-Object fileSize -Sum).Sum) / 1MB, 1)) MB"
""
"### partitions ###"
for ($p = 1; $p -le 4; $p++) {
  $part = @($tasks | Where-Object { $_.partition -eq $p })
  "  part-$p : {0,3} files  {1,9:N1} MB  versions: {2}" -f $part.Count, ((($part | Measure-Object fileSize -Sum).Sum) / 1MB), (($part | ForEach-Object { $_.versions } | Sort-Object -Unique) -join ', ')
}
"written to $OutDir"
