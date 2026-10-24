# Build a ReoMods distribution set from a task manifest.
#
# For every task the script:
#   1. downloads the used file (or copies a local source jar) into
#      <OriginalsDir>  (sha1 verified when the manifest provides one)
#   2. applies constant-pool display renames (single "pair", a "pairs" array,
#      or an automatic Poly* -> Reo* rename derived from fabric.mod.json)
#   3. rewrites fabric.mod.json: name override, authors ["drako"],
#      description OneClient->ReoClient / Polyfrost->Reo, contact policy,
#      Mod Menu link removal
#   4. auto-patches the nested platform jars used by OneConfig (oneconfigv1)
#      and PolyPlus (polydium -> Reodium), or patches explicitly listed
#      "nestedEntries"
#   5. for PolyPlus, applies the ReoClient wordmark (three SVG assets + the
#      ONECLIENT/OneClient display strings + lang text), replaces the in-game
#      white-star badge with the 12px smooth chrome R (16x9 tab-list texture +
#      24x24 font atlas), and adds the Reo mod icon, so the result no longer
#      depends on the source jar already being hand-patched
#   6. always applies the PolyPlus featured-server bytecode patch (auto for id
#      "polyplus"; task."featuredServer": true remains supported)
#   7. normalizes the jar and validates it before publishing to <OutDir>
#
# usage:
#   powershell -ExecutionPolicy Bypass -File build_reomods.ps1 -TasksPath <manifest.json>
#   powershell -ExecutionPolicy Bypass -File build_reomods.ps1 -TasksPath <manifest.json> -WorkDir <dir> -OutDir <dir>
param(
  [Parameter(Mandatory = $true)][string]$TasksPath,
  [string]$OutDir = "C:\Users\drako\Documents\Default Project\ReoMods",
  [string]$OriginalsDir = "",
  [string]$WorkDir = "C:\Users\drako\AppData\Local\Temp\opencode\reomods_build",
  [string]$WordmarkDir = ""
)

$ErrorActionPreference = 'Stop'
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolkitDir = Split-Path -Parent $scriptDir
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
if (-not $WordmarkDir) { $WordmarkDir = Join-Path $toolkitDir 'assets' }

# Branded PolyPlus assets, already produced by gen_assets.ps1 + finalize.ps1:
#   assets/reo.svg          = REOCLIENT wordmark (shipped as brand/oneclient.svg)
#   assets/logo.svg         = chrome R main-menu mark
#   assets/footer-logo.svg  = small R footer mark
#   assets/icon.png         = 256x256 Reo app icon for the mod list / Mod Menu
$wordmarkAssets = @(
  @{ Entry = 'assets/polyplus/brand/oneclient.svg';          File = 'reo.svg' },
  @{ Entry = 'assets/polyplus/mainmenu/logo.svg';            File = 'logo.svg' },
  @{ Entry = 'assets/polyplus/mainmenu/footer-logo.svg';     File = 'footer-logo.svg' }
)

# ReoPlus in-game badge, produced by gen_badge.ps1:
#   assets/badge.png             = 64x36 chrome R used by PolyPlusBadge.blitTab
#                                  (16x9 at 4x)
#   assets/badge.png.mcmeta      = no blur; the tab texture is drawn 1:1
#   assets/badge_font.png        = 24x24 chrome R bitmap-font glyph
#   assets/badge_font.png.mcmeta = linear filtering for the 12px nametag glyph
#   assets/badge.json            = one \uE000 glyph at height 12, ascent 11
$badgeAssets = @(
  @{ Entry = 'assets/polyplus/textures/badge.png';             File = 'badge.png' },
  @{ Entry = 'assets/polyplus/textures/badge.png.mcmeta';      File = 'badge.png.mcmeta' },
  @{ Entry = 'assets/polyplus/textures/badge_font.png';        File = 'badge_font.png' },
  @{ Entry = 'assets/polyplus/textures/badge_font.png.mcmeta'; File = 'badge_font.png.mcmeta' },
  @{ Entry = 'assets/polyplus/font/badge.json';                File = 'badge.json' }
)

if (-not $OriginalsDir) { $OriginalsDir = Join-Path $OutDir '_originals' }
New-Item -ItemType Directory -Path $OutDir, $OriginalsDir, $WorkDir -Force | Out-Null

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$jdkRoot = Join-Path $env:APPDATA 'drako\ReoLauncher-dev\metadata\java'
$javac = Get-ChildItem $jdkRoot -Recurse -Filter javac.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $javac) { throw "javac.exe not found under $jdkRoot" }
$java = Join-Path $javac.DirectoryName 'java.exe'
$javap = Join-Path $javac.DirectoryName 'javap.exe'

$cpToolDir = Join-Path $WorkDir 'cptool'
New-Item -ItemType Directory -Path $cpToolDir -Force | Out-Null
& $javac -d $cpToolDir (Join-Path $scriptDir 'CpTool.java')
if ($LASTEXITCODE -ne 0) { throw "javac CpTool.java failed" }

# ASM-based Reo cosmetics patch (all catalog entries owned, prices zeroed).
$cosmeticsToolDir = Join-Path $WorkDir 'cosmetics'
New-Item -ItemType Directory -Path $cosmeticsToolDir -Force | Out-Null
$asmJar = Join-Path $scriptDir 'asm-9.11.1.jar'
$asmTreeJar = Join-Path $scriptDir 'asm-tree-9.11.1.jar'
if (-not (Test-Path -LiteralPath $asmJar)) { throw "ASM jar missing: $asmJar" }
if (-not (Test-Path -LiteralPath $asmTreeJar)) { throw "ASM tree jar missing: $asmTreeJar" }
& $javac -cp $asmJar -d $cosmeticsToolDir (Join-Path $scriptDir 'ReoCosmeticsFree.java')
if ($LASTEXITCODE -ne 0) { throw "javac ReoCosmeticsFree.java failed" }

# ASM-based badge resize (class constants + the five badge assets).  The
# PolyPlus build starts from the pristine source jar, so this pass is what
# turns its 8px white-star glyph into the 12px smooth chrome R.
$badgeToolDir = Join-Path $WorkDir 'badge'
New-Item -ItemType Directory -Path $badgeToolDir -Force | Out-Null
& $javac -cp "$asmJar;$asmTreeJar" -d $badgeToolDir (Join-Path $scriptDir 'ReoBadgeSize.java')
if ($LASTEXITCODE -ne 0) { throw "javac ReoBadgeSize.java failed" }

# ASM-based local-equip fallback.  The backend rejects cosmetics the account
# does not own; this keeps the item equipped locally and lets the normal
# wardrobe/sync path continue instead of aborting on the HTTP failure.
$localEquipToolDir = Join-Path $WorkDir 'local-equip'
New-Item -ItemType Directory -Path $localEquipToolDir -Force | Out-Null
& $javac -cp $asmJar -d $localEquipToolDir (Join-Path $scriptDir 'ReoCosmeticsLocalEquip.java')
if ($LASTEXITCODE -ne 0) { throw "javac ReoCosmeticsLocalEquip.java failed" }

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

function Try-GetZipEntryBytes([string]$jar, [string]$name) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $entry = $zip.GetEntry($name)
    if (-not $entry) { return $null }
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

function Assert-JarNames([string]$jar) {
  $zip = [System.IO.Compression.ZipFile]::OpenRead($jar)
  try {
    $bad = @($zip.Entries | Where-Object { $_.FullName.Contains('\') })
    if ($zip.Entries.Count -eq 0) { throw "empty jar: $jar" }
    if ($bad.Count -gt 0) { throw "jar contains backslash entry names: $jar" }
    return $zip.Entries.Count
  } finally { $zip.Dispose() }
}

function Read-JsonFromBytes([byte[]]$bytes) {
  if (-not $bytes) { return $null }
  return ([System.Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json)
}

function Get-TaskSource($t) {
  if ($t.PSObject.Properties['sourceJar'] -and $t.sourceJar) {
    if (-not (Test-Path -LiteralPath $t.sourceJar)) { throw "sourceJar not found: $($t.sourceJar)" }
    return $t.sourceJar
  }
  $dest = Join-Path $OriginalsDir $t.usedName
  if (Test-Path -LiteralPath $dest) {
    if (-not $t.sha1) { return $dest }
    $h = (Get-FileHash -Algorithm SHA1 -LiteralPath $dest).Hash.ToLower()
    if ($h -eq $t.sha1.ToLower()) { return $dest }
    Write-Warning "cached original hash mismatch for $($t.usedName); re-downloading"
  }
  $tmp = "$dest.part"
  Remove-Item -LiteralPath $tmp -Force -ErrorAction SilentlyContinue
  & curl.exe -L --fail --silent --show-error -A 'ReoLauncher/dev (github.com/reomc)' -o $tmp $t.url
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $tmp)) {
    Invoke-WebRequest -Uri $t.url -UserAgent 'ReoLauncher/dev (github.com/reomc)' -OutFile $tmp -UseBasicParsing
  }
  if (-not (Test-Path -LiteralPath $tmp)) { throw "download failed: $($t.url)" }
  if ($t.sha1) {
    $h = (Get-FileHash -Algorithm SHA1 -LiteralPath $tmp).Hash.ToLower()
    if ($h -ne $t.sha1.ToLower()) { throw "sha1 mismatch for $($t.usedName): got $h" }
  }
  Move-Item -LiteralPath $tmp -Destination $dest -Force
  return $dest
}

function Invoke-CpApply([string]$inJar, [string]$outJar, [string[]]$pairs, [string]$caseDir) {
  $pairsPath = Join-Path $caseDir 'pairs.txt'
  [System.IO.File]::WriteAllText($pairsPath, (($pairs -join "`n") + "`n"), $utf8NoBom)
  & $java -cp $cpToolDir CpTool --apply $inJar $outJar $pairsPath
  if ($LASTEXITCODE -ne 0) { throw "CpTool --apply failed for $inJar" }
}

function Get-BytesSha1([byte[]]$bytes) {
  $sha = [System.Security.Cryptography.SHA1]::Create()
  try { return [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace('-', '').ToLowerInvariant() }
  finally { $sha.Dispose() }
}

# Replaces the three branded PolyPlus SVG assets and the lang display text.
# The class constants are handled by CpTool --apply-branding.
function Add-PolyPlusWordmark([string]$stage) {
  foreach ($a in $wordmarkAssets) {
    $assetPath = Join-Path $WordmarkDir $a.File
    if (-not (Test-Path -LiteralPath $assetPath)) { throw "wordmark asset missing: $assetPath" }
    Set-ZipEntryBytes $stage $a.Entry ([System.IO.File]::ReadAllBytes($assetPath))
  }
  $lang = Try-GetZipEntryBytes $stage 'assets/polyplus/lang/en_us.json'
  if ($lang) {
    # Values only: the key "polyplus.mainmenu.switchToOneClient" must stay intact.
    $text = [System.Text.Encoding]::UTF8.GetString($lang)
    $text = $text.Replace('"OneClient Menu"', '"ReoClient Menu"').Replace('OneClient/PolyPlus note:', 'ReoClient/PolyPlus note:')
    Set-ZipEntryBytes $stage 'assets/polyplus/lang/en_us.json' ($utf8NoBom.GetBytes($text))
  }
}

# Replaces the white PolyPlus star badge with the chrome R in both render paths:
# the 64x36 tab-list texture and the 24x24 bitmap-font glyph.  The font JSON and
# both mcmeta files are shipped too, so the 12px glyph gets linear filtering.
function Add-PolyPlusBadge([string]$stage) {
  foreach ($a in $badgeAssets) {
    $assetPath = Join-Path $WordmarkDir $a.File
    if (-not (Test-Path -LiteralPath $assetPath)) { throw "badge asset missing: $assetPath" }
    Set-ZipEntryBytes $stage $a.Entry ([System.IO.File]::ReadAllBytes($assetPath))
  }
}

# Rewrites PolyPlusBadge's 8px font/9x8 tab constants to the 12px/16x9 set and
# makes the tab-offset 19px.  Idempotent, so it also accepts jars that already
# contain the branded badge.
function Set-ReoBadgeSize([string]$stage, [string]$caseDir) {
  $badgeStage = Join-Path $caseDir 'stage-badge.jar'
  Remove-Item -LiteralPath $badgeStage -Force -ErrorAction SilentlyContinue
  & $java -cp "$badgeToolDir;$asmJar;$asmTreeJar" ReoBadgeSize $stage $badgeStage $WordmarkDir
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $badgeStage)) {
    throw "badge size patch failed"
  }
  Move-Item -LiteralPath $badgeStage -Destination $stage -Force
}

# Adds the Reo app icon to the mod jar and points fabric.mod.json at it.
# Upstream PolyPlus ships no icon, which makes Mod Menu fall back to a blank
# placeholder.  This runs after the fabric.mod.json rewrite so the field is not
# dropped by rebrand_fabric_mod.js.
function Add-PolyPlusIcon([string]$stage) {
  $iconPath = Join-Path $WordmarkDir 'icon.png'
  if (-not (Test-Path -LiteralPath $iconPath)) { throw "icon asset missing: $iconPath" }
  Set-ZipEntryBytes $stage 'assets/polyplus/icon.png' ([System.IO.File]::ReadAllBytes($iconPath))
  $fmj = Read-JsonFromBytes (Get-ZipEntryBytes $stage 'fabric.mod.json')
  $fmj | Add-Member -NotePropertyName icon -NotePropertyValue 'assets/polyplus/icon.png' -Force
  $json = $fmj | ConvertTo-Json -Depth 100 -Compress
  Set-ZipEntryBytes $stage 'fabric.mod.json' ($utf8NoBom.GetBytes($json))
}

function Test-PolyPlusWordmark([string]$outJar) {
  $oneHits = @(& $java -cp $cpToolDir CpTool --scan $outJar ONECLIENT 2>&1)
  $reoHits = @(& $java -cp $cpToolDir CpTool --scan $outJar REOCLIENT 2>&1)
  if ($oneHits.Count -gt 0) { throw "ONECLIENT still present in $outJar" }
  if ($reoHits.Count -eq 0) { throw "REOCLIENT missing from $outJar" }
  foreach ($a in $wordmarkAssets) {
    $actual = Get-BytesSha1 (Get-ZipEntryBytes $outJar $a.Entry)
    $expected = Get-BytesSha1 ([System.IO.File]::ReadAllBytes((Join-Path $WordmarkDir $a.File)))
    if ($actual -ne $expected) { throw "wordmark asset mismatch: $($a.Entry)" }
  }
}

function Test-PolyPlusBadge([string]$outJar) {
  foreach ($a in $badgeAssets) {
    $actual = Get-BytesSha1 (Get-ZipEntryBytes $outJar $a.Entry)
    $expected = Get-BytesSha1 ([System.IO.File]::ReadAllBytes((Join-Path $WordmarkDir $a.File)))
    if ($actual -ne $expected) { throw "badge asset mismatch: $($a.Entry)" }
  }
  $badgeJson = Read-JsonFromBytes (Get-ZipEntryBytes $outJar 'assets/polyplus/font/badge.json')
  $bitmap = @($badgeJson.providers | Where-Object { $_.type -eq 'bitmap' }) | Select-Object -First 1
  if (-not $bitmap -or $bitmap.file -ne 'polyplus:badge_font.png' -or
      [int]$bitmap.height -ne 12 -or [int]$bitmap.ascent -ne 11) {
    throw "badge font provider changed unexpectedly in $outJar"
  }
  $chars = @($bitmap.chars)
  if ($chars.Count -ne 1 -or [int][char]$chars[0] -ne 0xE000) {
    throw "badge font no longer maps the single E000 glyph in $outJar"
  }
  $fontMeta = Read-JsonFromBytes (Get-ZipEntryBytes $outJar 'assets/polyplus/textures/badge_font.png.mcmeta')
  if (-not $fontMeta.texture -or $fontMeta.texture.blur -ne $true) {
    throw "badge font texture is not configured for smooth filtering in $outJar"
  }

  $badgeClass = & $javap -p -constants -cp $outJar org.polyfrost.polyplus.client.PolyPlusBadge 2>&1
  foreach ($needle in @('TEX_W = 64', 'TEX_H = 36', 'TEXT_H = 12', 'BADGE_H = 9', 'BADGE_W = 16', 'BADGE_ADVANCE = 15')) {
    if (@($badgeClass | Select-String -SimpleMatch $needle).Count -eq 0) {
      throw "badge constant missing in $outJar : $needle"
    }
  }
  $tabClass = & $javap -p -c -cp $outJar org.polyfrost.polyplus.mixin.client.MixinPlayerTabOverlay 2>&1
  if (@($tabClass | Select-String -Pattern 'bipush\s+19').Count -eq 0) {
    throw "tab badge offset is not 19px in $outJar"
  }
}

function Edit-FabricModJson([string]$inPath, [string]$outPath, [string]$name, [string]$contact) {
  $nodeArgs = @((Join-Path $scriptDir 'rebrand_fabric_mod.js'), $inPath, $outPath)
  if ($name) { $nodeArgs += "--name=$name" }
  $nodeArgs += '--authors=drako'
  $nodeArgs += "--contact=$contact"
  & node @nodeArgs
  if ($LASTEXITCODE -ne 0) { throw "node rebrand_fabric_mod.js failed for $inPath" }
}

# Bytecode-only PolyPlus featured-server patch: make the client-side catalog
# loaders take the development-environment branch so
# -Dpolyplus.featuredServers.file is honoured.  Replaces
#   invokeinterface FabricLoader.isDevelopmentEnvironment()
# with
#   pop; iconst_1; nop; nop; nop
# (identical stack effect, same bytecode length, no frame changes).
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
  if ($wanted.Count -eq 0) { return [pscustomobject]@{ Bytes = $bytes; Count = 0; Reason = 'method-name-not-found' } }

  $natMatches = @{}
  foreach ($k in $natNameIdx.Keys) {
    if ($wanted.ContainsKey($natNameIdx[$k])) { $natMatches[$k] = $true }
  }
  $targets = @{}
  foreach ($k in $imrefNatIdx.Keys) {
    if ($natMatches.ContainsKey($imrefNatIdx[$k])) { $targets[$k] = $true }
  }
  if ($targets.Count -eq 0) { return [pscustomobject]@{ Bytes = $bytes; Count = 0; Reason = 'methodref-not-found' } }

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
  return [pscustomobject]@{ Bytes = $bytes; Count = $patched; Reason = '' }
}

function Add-FeaturedServerPatch([string]$jar) {
  $entry = 'org/polyfrost/polyplus/client/featured/FeaturedServers.class'
  $bytes = Try-GetZipEntryBytes $jar $entry
  if (-not $bytes) {
    Write-Warning "featured-server class not found; skipping patch"
    return 0
  }
  $result = Set-DevCatalogPath $bytes
  if ($result.Count -gt 0) {
    Set-ZipEntryBytes $jar $entry $result.Bytes
    return $result.Count
  }
  Write-Warning "featured-server patch found no call sites (code $($result.Count), $($result.Reason))"
  return 0
}

# Nested platform jars that carry their own display identity.
function Get-NestedFmjSpec($nfmj) {
  if (-not $nfmj -or -not $nfmj.PSObject.Properties['id']) { return $null }
  if ($nfmj.id -eq 'polydium') {
    return [pscustomobject]@{ name = 'Reodium'; contact = 'remove'; pairs = @('Polydium=>Reodium'); strictPairs = $false }
  }
  if ($nfmj.id -eq 'oneconfigv1') {
    return [pscustomobject]@{ name = $null; contact = 'discord'; pairs = @(); strictPairs = $false }
  }
  return $null
}

function Edit-AutoNested([string]$stage, $task, [string]$caseDir) {
  $touched = @()
  $outerFmj = Read-JsonFromBytes (Get-ZipEntryBytes $stage 'fabric.mod.json')
  $entries = @()
  if ($outerFmj.PSObject.Properties['jars'] -and $outerFmj.jars) {
    foreach ($j in @($outerFmj.jars)) {
      if ($j.PSObject.Properties['file'] -and $j.file) { $entries += $j.file }
    }
  }
  foreach ($entryName in ($entries | Select-Object -Unique)) {
    if ($entryName -notmatch '^META-INF/jars/.+\.jar$') { continue }
    $nestedIn = Join-Path $caseDir 'nested-in.jar'
    [System.IO.File]::WriteAllBytes($nestedIn, (Get-ZipEntryBytes $stage $entryName))
    $nfmjBytes = Try-GetZipEntryBytes $nestedIn 'fabric.mod.json'
    if (-not $nfmjBytes) { continue }
    $nfmj = Read-JsonFromBytes $nfmjBytes
    $spec = Get-NestedFmjSpec $nfmj
    if (-not $spec) { continue }

    $nestedStage = Join-Path $caseDir 'nested-stage.jar'
    if (@($spec.pairs).Count -gt 0) {
      Invoke-CpApply $nestedIn $nestedStage @($spec.pairs) $caseDir
    } else {
      Copy-Item -LiteralPath $nestedIn -Destination $nestedStage -Force
    }

    $nfmjIn = Join-Path $caseDir 'nested-fmj-in.json'
    $nfmjOut = Join-Path $caseDir 'nested-fmj-out.json'
    [System.IO.File]::WriteAllBytes($nfmjIn, (Get-ZipEntryBytes $nestedStage 'fabric.mod.json'))
    Edit-FabricModJson $nfmjIn $nfmjOut $spec.name $spec.contact
    Set-ZipEntryBytes $nestedStage 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($nfmjOut))

    $nestedOut = Join-Path $caseDir 'nested-out.jar'
    & $java -cp $cpToolDir CpTool --normalize $nestedStage $nestedOut
    if ($LASTEXITCODE -ne 0) { throw "nested normalize failed: $entryName" }

    $nfmjFinal = Read-JsonFromBytes (Get-ZipEntryBytes $nestedOut 'fabric.mod.json')
    if (@($nfmjFinal.authors).Count -ne 1 -or $nfmjFinal.authors[0] -ne 'drako') { throw "nested authors not drako: $entryName" }
    if ($spec.name -and $nfmjFinal.name -ne $spec.name) { throw "nested name mismatch $entryName : $($nfmjFinal.name)" }
    if ($spec.contact -eq 'remove' -and $nfmjFinal.contact) { throw "nested contact not removed: $entryName" }
    if ($spec.contact -eq 'discord' -and (-not $nfmjFinal.contact -or $nfmjFinal.contact.homepage -ne 'https://discord.gg/reomc')) { throw "nested contact not discord: $entryName" }
    if (@($spec.pairs).Count -gt 0) {
      foreach ($p in @($spec.pairs)) {
        $sep = $p.IndexOf('=>'); $oldName = $p.Substring(0, $sep); $newName = $p.Substring($sep + 2)
        $oldHits = @(& $java -cp $cpToolDir CpTool --scan $nestedOut $oldName 2>&1)
        if ($oldHits.Count -gt 0) { throw "nested old display name still present $entryName : $oldName" }
      }
    }

    Set-ZipEntryBytes $stage $entryName ([System.IO.File]::ReadAllBytes($nestedOut))
    $touched += $entryName
  }
  return $touched
}

function Edit-ListedNestedEntries([string]$stage, $task, [string]$caseDir) {
  $touched = @()
  foreach ($ne in @($task.nestedEntries)) {
    $entryName = $ne.entry
    $nestedIn = Join-Path $caseDir 'nested-in.jar'
    [System.IO.File]::WriteAllBytes($nestedIn, (Get-ZipEntryBytes $stage $entryName))
    # Explicitly listed nested jars still get the platform policy (polydium ->
    # Reodium class strings, oneconfigv1 -> Discord contact).
    $nestedSpec = Get-NestedFmjSpec (Read-JsonFromBytes (Get-ZipEntryBytes $nestedIn 'fabric.mod.json'))
    if ($nestedSpec -and @($nestedSpec.pairs).Count -gt 0) {
      $nestedStage = Join-Path $caseDir 'nested-stage.jar'
      Invoke-CpApply $nestedIn $nestedStage @($nestedSpec.pairs) $caseDir
      $nestedIn = $nestedStage
    }
    $nestedFmj = Join-Path $caseDir 'nested-fmj-in.json'
    [System.IO.File]::WriteAllBytes($nestedFmj, (Get-ZipEntryBytes $nestedIn 'fabric.mod.json'))
    $nestedFmjOut = Join-Path $caseDir 'nested-fmj-out.json'
    $nestedName = $null
    $nestedContact = 'keep'
    if ($nestedSpec) { $nestedName = $nestedSpec.name; $nestedContact = $nestedSpec.contact }
    if ($ne.PSObject.Properties['name'] -and $ne.name) { $nestedName = $ne.name }
    if ($ne.PSObject.Properties['contact'] -and $ne.contact) { $nestedContact = $ne.contact }
    Edit-FabricModJson $nestedFmj $nestedFmjOut $nestedName $nestedContact
    Set-ZipEntryBytes $nestedIn 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($nestedFmjOut))
    $nestedOut = Join-Path $caseDir 'nested-out.jar'
    & $java -cp $cpToolDir CpTool --normalize $nestedIn $nestedOut
    if ($LASTEXITCODE -ne 0) { throw "nested normalize failed: $entryName" }
    $nfmj = Read-JsonFromBytes (Get-ZipEntryBytes $nestedOut 'fabric.mod.json')
    if (@($nfmj.authors).Count -ne 1 -or $nfmj.authors[0] -ne 'drako') { throw "nested authors not drako: $entryName" }
    if ($nestedName -and $nfmj.name -ne $nestedName) { throw "nested name mismatch $entryName : $($nfmj.name)" }
    if ($nestedSpec -and @($nestedSpec.pairs).Count -gt 0) {
      foreach ($p in @($nestedSpec.pairs)) {
        $sep = $p.IndexOf('=>'); $pOld = $p.Substring(0, $sep)
        $oldHits = @(& $java -cp $cpToolDir CpTool --scan $nestedOut $pOld 2>&1)
        if ($oldHits.Count -gt 0) { throw "nested old display name still present $entryName : $pOld" }
      }
    }
    Set-ZipEntryBytes $stage $entryName ([System.IO.File]::ReadAllBytes($nestedOut))
    $touched += $entryName
  }
  return $touched
}

$tasks = Get-Content -LiteralPath $TasksPath -Raw | ConvertFrom-Json
if ($tasks -isnot [array]) { $tasks = @($tasks) }

$results = @()
$failures = @()
foreach ($t in $tasks) {
  try {
    Write-Host "== $($t.slug) / $($t.usedName) =="
    $src = Get-TaskSource $t
    $caseKey = [System.IO.Path]::GetFileNameWithoutExtension($t.usedName) -replace '[^A-Za-z0-9._+-]', '_'
    $caseDir = Join-Path $WorkDir "$($t.slug)-$caseKey"
    New-Item -ItemType Directory -Path $caseDir -Force | Out-Null
    $inJar = Join-Path $caseDir 'in.jar'
    Copy-Item -LiteralPath $src -Destination $inJar -Force

    $inFmj = Read-JsonFromBytes (Get-ZipEntryBytes $inJar 'fabric.mod.json')
    $oldName = [string]$inFmj.name
    $targetName = $null
    if ($t.PSObject.Properties['name'] -and $t.name) {
      $targetName = [string]$t.name
    } elseif ($t.PSObject.Properties['autoPolyName'] -and $t.autoPolyName -and $oldName -like 'Poly*') {
      $targetName = 'Reo' + $oldName.Substring(4)
    }

    $pairsList = @()
    if ($t.PSObject.Properties['pairs'] -and $t.pairs) { $pairsList += @($t.pairs) }
    if ($t.PSObject.Properties['pair'] -and $t.pair) { $pairsList += [string]$t.pair }
    if ($t.PSObject.Properties['autoPolyName'] -and $t.autoPolyName -and $targetName -and $targetName -ne $oldName) {
      $pairsList += "$oldName=>$targetName"
    }
    if ($inFmj.id -eq 'polyplus') {
      # Covers pristine ("OneClient") and previously wordmark-patched ("ReoClient") sources.
      $pairsList += 'PolyPlus (OneClient)=>ReoPlus (ReoClient)'
      $pairsList += 'PolyPlus (ReoClient)=>ReoPlus (ReoClient)'
    }
    $pairsList = @($pairsList | Where-Object { $_ } | Select-Object -Unique)

    $stage = Join-Path $caseDir 'stage.jar'
    if ($pairsList.Count -gt 0) {
      Invoke-CpApply $inJar $stage $pairsList $caseDir
    } else {
      Copy-Item -LiteralPath $inJar -Destination $stage -Force
    }

    $isPolyPlus = $inFmj.id -eq 'polyplus'
    if ($isPolyPlus) {
      # Wordmark pass: ONECLIENT/OneClient display strings + the three SVG assets,
      # then replace the white star badge with the chrome R badge assets.
      $brandStage = Join-Path $caseDir 'stage-branding.jar'
      Remove-Item -LiteralPath $brandStage -Force -ErrorAction SilentlyContinue
      & $java -cp $cpToolDir CpTool --apply-branding $stage $brandStage
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $brandStage)) {
        throw "wordmark class patch failed for $($t.slug)"
      }
      Move-Item -LiteralPath $brandStage -Destination $stage -Force
      Add-PolyPlusWordmark $stage
      Add-PolyPlusBadge $stage
      Set-ReoBadgeSize $stage $caseDir
    }

    $contact = if ($t.PSObject.Properties['contact'] -and $t.contact) { [string]$t.contact } else { 'remove' }
    $fmjIn = Join-Path $caseDir 'fmj-in.json'
    $fmjOut = Join-Path $caseDir 'fmj-out.json'
    [System.IO.File]::WriteAllBytes($fmjIn, (Get-ZipEntryBytes $stage 'fabric.mod.json'))
    Edit-FabricModJson $fmjIn $fmjOut $targetName $contact
    Set-ZipEntryBytes $stage 'fabric.mod.json' ([System.IO.File]::ReadAllBytes($fmjOut))

    $nestedTouched = @()
    if ($t.PSObject.Properties['nestedAuto'] -and $t.nestedAuto) {
      $nestedTouched = Edit-AutoNested $stage $t $caseDir
    }
    if ($t.PSObject.Properties['nestedEntries'] -and @($t.nestedEntries).Count -gt 0) {
      $nestedTouched += Edit-ListedNestedEntries $stage $t $caseDir
    }

    $featuredSites = 0
    if ($isPolyPlus -or ($t.PSObject.Properties['featuredServer'] -and $t.featuredServer)) {
      $featuredSites = Add-FeaturedServerPatch $stage
    }

    $cosmeticsFree = $false
    if ($isPolyPlus) {
      $freeStage = Join-Path $caseDir 'stage-cosmetics.jar'
      Remove-Item -LiteralPath $freeStage -Force -ErrorAction SilentlyContinue
      & $java -cp "$cosmeticsToolDir;$asmJar" ReoCosmeticsFree $stage $freeStage
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $freeStage)) {
        throw "cosmetics patch failed for $($t.slug)"
      }
      Move-Item -LiteralPath $freeStage -Destination $stage -Force
      $cosmeticsFree = $true

      # Local-equip fallback: the backend rejects unowned items, but the client
      # must still apply the wardrobe selection and finish the sync path.
      $helperDir = Join-Path $caseDir 'local-equip'
      New-Item -ItemType Directory -Path $helperDir -Force | Out-Null
      & $javac -cp $stage -d $helperDir (Join-Path $scriptDir 'ReoLocalEquip.java')
      if ($LASTEXITCODE -ne 0) { throw "javac ReoLocalEquip.java failed for $($t.slug)" }
      $helperClass = Join-Path $helperDir 'org\polyfrost\polyplus\client\cosmetics\ReoLocalEquip.class'
      if (-not (Test-Path -LiteralPath $helperClass)) { throw "ReoLocalEquip.class missing for $($t.slug)" }
      $localStage = Join-Path $caseDir 'stage-local-equip.jar'
      Remove-Item -LiteralPath $localStage -Force -ErrorAction SilentlyContinue
      & $java -cp "$localEquipToolDir;$asmJar" ReoCosmeticsLocalEquip $stage $localStage $helperClass
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $localStage)) {
        throw "local-equip patch failed for $($t.slug)"
      }
      Move-Item -LiteralPath $localStage -Destination $stage -Force
      Add-PolyPlusIcon $stage
    }

    $outJar = Join-Path $caseDir 'out.jar'
    & $java -cp $cpToolDir CpTool --normalize $stage $outJar
    if ($LASTEXITCODE -ne 0) { throw "CpTool --normalize failed for $($t.slug)" }

    $inCount = Assert-JarNames $inJar
    $outCount = Assert-JarNames $outJar
    $expectedOutCount = $inCount
    if ($isPolyPlus) {
      $expectedOutCount += 2 # assets/polyplus/icon.png + ReoLocalEquip.class
      foreach ($a in $badgeAssets) {
        if (-not (Try-GetZipEntryBytes $inJar $a.Entry)) { $expectedOutCount++ }
      }
    }
    if ($outCount -ne $expectedOutCount) {
      throw "entry count changed for $($t.slug): $inCount -> $outCount (expected $expectedOutCount)"
    }

    $fmj = Read-JsonFromBytes (Get-ZipEntryBytes $outJar 'fabric.mod.json')
    if (@($fmj.authors).Count -ne 1 -or $fmj.authors[0] -ne 'drako') { throw "authors not drako for $($t.slug)" }
    if ($targetName -and $fmj.name -ne $targetName) { throw "name mismatch for $($t.slug): $($fmj.name)" }
    if ($contact -eq 'remove') {
      if ($fmj.contact) { throw "contact not removed for $($t.slug)" }
    } elseif ($contact -eq 'discord') {
      if (-not $fmj.contact -or $fmj.contact.homepage -ne 'https://discord.gg/reomc') { throw "contact not discord for $($t.slug)" }
    }
    if ($fmj.custom -and $fmj.custom.modmenu -and $fmj.custom.modmenu.links) { throw "modmenu links not removed for $($t.slug)" }

    $strictPairs = $true
    if ($t.PSObject.Properties['strictPairs'] -and $t.strictPairs -eq $false) { $strictPairs = $false }
    if ($pairsList.Count -gt 0) {
      $autoPair = if ($t.PSObject.Properties['autoPolyName'] -and $t.autoPolyName -and $targetName) { "$oldName=>$targetName" } else { $null }
      foreach ($p in $pairsList) {
        $sep = $p.IndexOf('=>')
        if ($sep -lt 0) { throw "bad pair: $p" }
        $pOld = $p.Substring(0, $sep); $pNew = $p.Substring($sep + 2)
        $oldHits = @(& $java -cp $cpToolDir CpTool --scan $outJar $pOld 2>&1)
        $newHits = @(& $java -cp $cpToolDir CpTool --scan $outJar $pNew 2>&1)
        if ($oldHits.Count -gt 0) { throw "old display name still present in $($t.slug): $pOld" }
        if ($strictPairs -or $p -eq $autoPair) {
          if ($newHits.Count -eq 0) { throw "new display name not found in $($t.slug): $pNew" }
        }
      }
    }
    if ($isPolyPlus) {
      Test-PolyPlusWordmark $outJar
      Test-PolyPlusBadge $outJar
      $zip = [System.IO.Compression.ZipFile]::OpenRead($outJar)
      try {
        $localHelper = $zip.GetEntry('org/polyfrost/polyplus/client/cosmetics/ReoLocalEquip.class')
        $iconEntry = $zip.GetEntry('assets/polyplus/icon.png')
      } finally { $zip.Dispose() }
      if (-not $localHelper) { throw "local-equip helper missing for $($t.slug)" }
      if (-not $iconEntry) { throw "mod icon missing for $($t.slug)" }
      if ($fmj.icon -ne 'assets/polyplus/icon.png') { throw "mod icon field wrong for $($t.slug): $($fmj.icon)" }
      $javapOut = & $javap -c -p -cp $outJar org.polyfrost.polyplus.client.featured.FeaturedServers 2>&1
      if (@($javapOut | Select-String 'isDevelopmentEnvironment').Count -ne 0) {
        throw "featured-server bytecode patch not applied for $($t.slug)"
      }
    }
    & $java -cp $cpToolDir CpTool --scan $outJar ReoClient | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "jar is not stream-readable: $($t.slug)" }

    $destJar = Join-Path $OutDir $t.outName
    Copy-Item -LiteralPath $outJar -Destination $destJar -Force
    $results += [pscustomobject]@{
      Slug     = $t.slug
      File     = $t.outName
      Name     = [string]$fmj.name
      Entries  = $outCount
      Nested   = ($nestedTouched -join ', ')
      Featured = $featuredSites
      Cosmetics = $cosmeticsFree
      Sha1     = (Get-FileHash -Algorithm SHA1 -LiteralPath $destJar).Hash.ToLower()
      Sha512   = (Get-FileHash -Algorithm SHA512 -LiteralPath $destJar).Hash.ToLower()
      Bytes    = (Get-Item -LiteralPath $destJar).Length
    }
    Write-Host "   ok -> $($t.outName)"
  } catch {
    Write-Warning "FAILED $($t.slug) / $($t.usedName): $($_.Exception.Message)"
    $failures += [pscustomobject]@{ Slug = $t.slug; File = $t.usedName; Error = $_.Exception.Message }
  }
}

""
"=== results ==="
$results | Format-Table -AutoSize -Wrap | Out-String -Width 240
"published: $($results.Count)  failed: $($failures.Count)  out: $OutDir"
if ($failures.Count -gt 0) {
  $failures | Format-Table -AutoSize -Wrap | Out-String -Width 240
  exit 1
}
"done"
