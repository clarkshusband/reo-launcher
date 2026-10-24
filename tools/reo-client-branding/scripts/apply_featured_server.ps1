# Apply the ReoMC featured-server support to the installed PolyPlus jar.
#
# What it does
#   1. polyplus: "PolyPlus (ReoClient)" config title -> "ReoPlus (ReoClient)"
#   2. FeaturedServers.class: make the two client-side catalog loaders treat the
#      environment as a development environment (so -Dpolyplus.featuredServers.file
#      is honoured).  This is a 5-byte bytecode substitution per call site:
#         invokeinterface FabricLoader.isDevelopmentEnvironment()
#      becomes
#         pop; iconst_1; nop; nop; nop
#      Net stack effect is identical and bytecode offsets/frames do not change.
#   3. fabric.mod.json: display name PolyPlus -> ReoPlus, OneClient -> ReoClient,
#      contact links removed.
#   4. nested META-INF/jars/polydium-1.1.5+26.3.jar: display name Polydium -> Reodium,
#      description OneClient -> ReoClient, contact links removed.
#
# usage:
#   powershell -ExecutionPolicy Bypass -File apply_featured_server.ps1            (build only)
#   powershell -ExecutionPolicy Bypass -File apply_featured_server.ps1 -Swap      (install)
param(
  [string]$JarPath = "",
  [string]$InstallPath = "C:\Users\drako\AppData\Roaming\drako\ReoLauncher-dev\clusters\26.3 Fabric\mods\polyplus-1.3.10+26.3.jar",
  [string]$WorkDir = "C:\Users\drako\AppData\Local\Temp\opencode\featured_patch",
  [switch]$Swap
)

$ErrorActionPreference = 'Stop'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolkit = Split-Path -Parent $scriptDir
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
if (-not $JarPath) { $JarPath = Join-Path $toolkit 'out\polyplus-1.3.10+26.3.jar' }
if (-not (Test-Path -LiteralPath $JarPath)) { throw "base jar not found: $JarPath" }

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# ---------- locate a JDK ----------
$jdkRoot = Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\java'
$javac = Get-ChildItem $jdkRoot -Recurse -Filter javac.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $javac) { throw "javac.exe not found under $jdkRoot" }
$javaBin = $javac.DirectoryName
$java = Join-Path $javaBin 'java.exe'
$javap = Join-Path $javaBin 'javap.exe'

# ---------- helpers ----------
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
      if ($e.Name -eq '') { continue } # directory marker
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
    "  entries: $($zip.Entries.Count)  bad names: $($bad.Count)"
    if ($bad.Count -gt 0) { throw "jar contains backslash entry names" }
  } finally { $zip.Dispose() }
}

# Constant-pool-only string substitution for the two client-side catalog loaders.
function Set-DevCatalogPath([byte[]]$bytes) {
  $cpCount = ([int]$bytes[8] -shl 8) -bor [int]$bytes[9]
  $utf8 = @{}
  $natNameIdx = @{}
  $imrefNatIdx = @{}
  $pos = 10
  $i = 1
  while ($i -lt $cpCount) {
    $tag = [int]$bytes[$pos]; $pos++
    switch ($tag) {
      1 {
        $len = ([int]$bytes[$pos] -shl 8) -bor [int]$bytes[$pos + 1]
        $utf8[$i] = [System.Text.Encoding]::UTF8.GetString($bytes, $pos + 2, $len)
        $pos += 2 + $len
      }
      3 { $pos += 4 }
      4 { $pos += 4 }
      5 { $pos += 8; $i++ }
      6 { $pos += 8; $i++ }
      7 { $pos += 2 }
      8 { $pos += 2 }
      9 { $pos += 4 }
      10 { $pos += 4 }
      11 {
        $imrefNatIdx[$i] = ([int]$bytes[$pos + 2] -shl 8) -bor [int]$bytes[$pos + 3]
        $pos += 4
      }
      12 {
        $natNameIdx[$i] = ([int]$bytes[$pos] -shl 8) -bor [int]$bytes[$pos + 1]
        $pos += 4
      }
      15 { $pos += 3 }
      16 { $pos += 2 }
      17 { $pos += 4 }
      18 { $pos += 4 }
      19 { $pos += 2 }
      20 { $pos += 2 }
      default { throw "unknown constant pool tag $tag at index $i" }
    }
    $i++
  }

  $wanted = @{}
  foreach ($k in $utf8.Keys) {
    if ($utf8[$k] -eq 'isDevelopmentEnvironment') { $wanted[$k] = $true }
  }
  if ($wanted.Count -eq 0) { throw "isDevelopmentEnvironment not found in constant pool" }

  $natMatches = @{}
  foreach ($k in $natNameIdx.Keys) {
    if ($wanted.ContainsKey($natNameIdx[$k])) { $natMatches[$k] = $true }
  }
  $targets = @{}
  foreach ($k in $imrefNatIdx.Keys) {
    if ($natMatches.ContainsKey($imrefNatIdx[$k])) { $targets[$k] = $true }
  }
  if ($targets.Count -eq 0) { throw "interface methodref not found" }

  $patched = 0
  for ($p = 0; $p -le $bytes.Length - 5; $p++) {
    if ($bytes[$p] -ne 0xB9) { continue }
    if ($bytes[$p + 3] -ne 0x01 -or $bytes[$p + 4] -ne 0x00) { continue }
    $idx = ([int]$bytes[$p + 1] -shl 8) -bor [int]$bytes[$p + 2]
    if (-not $targets.ContainsKey($idx)) { continue }
    $bytes[$p] = 0x57      # pop  (drop FabricLoader instance)
    $bytes[$p + 1] = 0x04  # iconst_1 (pretend development environment)
    $bytes[$p + 2] = 0x00  # nop
    $bytes[$p + 3] = 0x00
    $bytes[$p + 4] = 0x00
    $patched++
  }
  if ($patched -eq 0) {
    Write-Warning "no unpatched isDevelopmentEnvironment call sites found (already patched?)"
    return $bytes
  }
  if ($patched -ne 2) { throw "expected 2 isDevelopmentEnvironment call sites, patched $patched" }
  return $bytes
}

function Edit-FabricModJson([string]$inPath, [string]$outPath, [string]$newName) {
  & node (Join-Path $scriptDir 'edit_fabric_mod.js') $inPath $outPath $newName
  if ($LASTEXITCODE -ne 0) { throw "node edit_fabric_mod.js failed" }
}

# ---------- workspace ----------
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$srcJar = Join-Path $WorkDir 'polyplus-in.jar'
Copy-Item -LiteralPath $JarPath -Destination $srcJar -Force

# compile the constant pool tool
$cpToolSrc = Join-Path $scriptDir 'CpTool.java'
$cpToolDir = Join-Path $WorkDir 'cptool'
New-Item -ItemType Directory -Path $cpToolDir -Force | Out-Null
& $javac -d $cpToolDir $cpToolSrc
if ($LASTEXITCODE -ne 0) { throw "javac CpTool.java failed" }
$java = Join-Path $javaBin 'java.exe'

# ---------- 1. display strings in polyplus classes ----------
$pairs = Join-Path $WorkDir 'polyplus.pairs'
[System.IO.File]::WriteAllText($pairs, "PolyPlus (ReoClient)=>ReoPlus (ReoClient)`nReoClient/PolyPlus note=>ReoClient/ReoPlus note`n", $utf8NoBom)
$stage1 = Join-Path $WorkDir 'polyplus-stage1.jar'
& $java -cp $cpToolDir CpTool --apply $srcJar $stage1 $pairs
if ($LASTEXITCODE -ne 0) { throw "CpTool failed" }

# ---------- 2. FeaturedServers bytecode ----------
$classEntry = 'org/polyfrost/polyplus/client/featured/FeaturedServers.class'
$classBytes = Get-ZipEntryBytes $stage1 $classEntry
$patchedBytes = Set-DevCatalogPath $classBytes
Set-ZipEntryBytes $stage1 $classEntry $patchedBytes
"patched FeaturedServers.class (2 call sites)"

# ---------- 3. polyplus fabric.mod.json ----------
$fmjIn = Join-Path $WorkDir 'polyplus-fmj-in.json'
$fmjOut = Join-Path $WorkDir 'polyplus-fmj-out.json'
[System.IO.File]::WriteAllBytes($fmjIn, (Get-ZipEntryBytes $stage1 'fabric.mod.json'))
Edit-FabricModJson $fmjIn $fmjOut 'ReoPlus'
Set-ZipEntryBytes $stage1 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($fmjOut))
"patched polyplus fabric.mod.json"

# ---------- 4. nested polydium ----------
$nestedName = 'META-INF/jars/polydium-1.1.5+26.3.jar'
$nestedJar = Join-Path $WorkDir 'polydium-in.jar'
[System.IO.File]::WriteAllBytes($nestedJar, (Get-ZipEntryBytes $stage1 $nestedName))
$pairs2 = Join-Path $WorkDir 'polydium.pairs'
[System.IO.File]::WriteAllText($pairs2, "Polydium=>Reodium`nOneClient=>ReoClient`n", $utf8NoBom)
$nestedOut = Join-Path $WorkDir 'polydium-out.jar'
& $java -cp $cpToolDir CpTool --apply $nestedJar $nestedOut $pairs2
if ($LASTEXITCODE -ne 0) { throw "CpTool failed for polydium" }
$pFmjIn = Join-Path $WorkDir 'polydium-fmj-in.json'
$pFmjOut = Join-Path $WorkDir 'polydium-fmj-out.json'
[System.IO.File]::WriteAllBytes($pFmjIn, (Get-ZipEntryBytes $nestedOut 'fabric.mod.json'))
Edit-FabricModJson $pFmjIn $pFmjOut 'Reodium'
Set-ZipEntryBytes $nestedOut 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($pFmjOut))
$nestedFinal = Join-Path $WorkDir 'polydium-final.jar'
& $java -cp $cpToolDir CpTool --normalize $nestedOut $nestedFinal
if ($LASTEXITCODE -ne 0) { throw "normalize polydium failed" }
Set-ZipEntryBytes $stage1 $nestedName ([System.IO.File]::ReadAllBytes($nestedFinal))
"patched nested polydium"

# ---------- 5. normalize + validate ----------
$outJar = Join-Path $WorkDir 'polyplus-1.3.10+26.3.jar'
& $java -cp $cpToolDir CpTool --normalize $stage1 $outJar
if ($LASTEXITCODE -ne 0) { throw "normalize polyplus failed" }
"built: $outJar ($((Get-Item $outJar).Length) bytes)"

"validation:"
Assert-JarNames $outJar
$before = Get-JarEntryMap $srcJar
$after = Get-JarEntryMap $outJar
$changed = @($after.Keys | Where-Object { -not $before.ContainsKey($_) -or $before[$_] -ne $after[$_] })
$changed += @($before.Keys | Where-Object { -not $after.ContainsKey($_) })
"  changed entries:"
$changed | Sort-Object -Unique | ForEach-Object { "    $_" }

& $java -cp $cpToolDir CpTool --scan $outJar ReoPlus | Out-Null
if ($LASTEXITCODE -ne 0) { throw "jar is not stream-readable (zip local headers)" }
"  stream-readable: yes"

$javapOut = & $javap -c -p -cp $outJar org.polyfrost.polyplus.client.featured.FeaturedServers 2>&1
if ($javapOut -match 'pop' -and $javapOut -match 'iconst_1') { "  bytecode check: pop/iconst_1 present" } else { throw "bytecode patch not visible in javap" }
$configOut = & $javap -c -p -cp $outJar org.polyfrost.polyplus.client.PolyPlusConfig 2>&1
if ($configOut -match 'ReoPlus \(ReoClient\)') { "  config title: ReoPlus (ReoClient)" } else { throw "config title not patched" }
$fmjText = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $outJar 'fabric.mod.json'))
if ($fmjText -notmatch '"name"\s*:\s*"ReoPlus"') { throw "fabric.mod.json name not patched" }
"  fabric.mod.json name: ReoPlus"
$nestedText = [System.Text.Encoding]::UTF8.GetString((Get-ZipEntryBytes $nestedFinal 'fabric.mod.json'))
if ($nestedText -notmatch '"name"\s*:\s*"Reodium"') { throw "nested fabric.mod.json name not patched" }
if ($nestedText -notmatch 'org\.polyfrost\.polydium\.client\.PolydiumClient') { throw "nested entrypoints corrupted" }
"  nested polydium name: Reodium (entrypoints intact)"

# ---------- 6. install ----------
if ($Swap) {
  $origHash = (Get-FileHash -Algorithm SHA1 -LiteralPath $InstallPath).Hash.ToLower()
  Remove-Item -LiteralPath $InstallPath -Force
  Copy-Item -LiteralPath $outJar -Destination $InstallPath -Force
  $newHash = (Get-FileHash -Algorithm SHA1 -LiteralPath $InstallPath).Hash.ToLower()
  "swapped into cluster: $origHash -> $newHash"
  cmd /c "fsutil hardlink list `"$InstallPath`""
}
"done"
