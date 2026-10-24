param(
  [string]$Base = "C:\Users\drako\AppData\Local\Temp\opencode\reo_client_patch",
  [switch]$Swap
)
$ErrorActionPreference = 'Stop'
$work = "$Base\work"
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

# ---------- 1. lang + fabric.mod.json ----------
$langPath = "$work\assets\polyplus\lang\en_us.json"
$lang = [System.IO.File]::ReadAllText($langPath, $utf8NoBom)
$lang2 = $lang.Replace('"OneClient Menu"', '"ReoClient Menu"').Replace('OneClient/PolyPlus note:', 'ReoClient/PolyPlus note:')
[System.IO.File]::WriteAllText($langPath, $lang2, $utf8NoBom)
"lang patched: $(@($lang2 -split "`n").Count) lines"

$fmjPath = "$work\fabric.mod.json"
$fmj = [System.IO.File]::ReadAllText($fmjPath, $utf8NoBom)
$fmj2 = $fmj.Replace('the OneClient ', 'the ReoClient ')
[System.IO.File]::WriteAllText($fmjPath, $fmj2, $utf8NoBom)
"fabric.mod.json patched: $($fmj2 -match 'ReoClient')"

# ---------- 2. svg assets ----------
Copy-Item "$Base\assets\oneclient.svg"     "$work\assets\polyplus\brand\oneclient.svg" -Force
Copy-Item "$Base\assets\logo.svg"          "$work\assets\polyplus\mainmenu\logo.svg" -Force
Copy-Item "$Base\assets\footer-logo.svg"   "$work\assets\polyplus\mainmenu\footer-logo.svg" -Force
"svg assets swapped"

# ---------- 3. repack (manual zip so entry names use '/' - .NET Framework CreateFromDirectory writes '\') ----------
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$outDir = "$Base\out"
New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$outJar = "$outDir\polyplus-1.3.10+26.3.jar"
Remove-Item $outJar -Force -ErrorAction SilentlyContinue
$rootPath = (Resolve-Path $work).Path
$rootLen = $rootPath.Length + 1
$fs = [System.IO.File]::Create($outJar)
$za = New-Object System.IO.Compression.ZipArchive($fs, [System.IO.Compression.ZipArchiveMode]::Create)
Get-ChildItem $rootPath -Recurse -Directory | ForEach-Object {
  $rel = $_.FullName.Substring($rootLen).Replace('\', '/') + '/'
  [void]$za.CreateEntry($rel)
}
Get-ChildItem $rootPath -Recurse -File | ForEach-Object {
  $rel = $_.FullName.Substring($rootLen).Replace('\', '/')
  $entry = $za.CreateEntry($rel, [System.IO.Compression.CompressionLevel]::Optimal)
  $es = $entry.Open()
  $bytes = [System.IO.File]::ReadAllBytes($_.FullName)
  $es.Write($bytes, 0, $bytes.Length)
  $es.Dispose()
}
$za.Dispose()
$fs.Dispose()
$zip = [System.IO.Compression.ZipFile]::OpenRead($outJar)
$bad = @($zip.Entries | Where-Object { $_.FullName.Contains('\') })
"repacked jar: $((Get-Item $outJar).Length) bytes, $($zip.Entries.Count) entries, bad-names: $($bad.Count)"
$zip.Dispose()

# ---------- 4. javap validation on patched classes ----------
$jdkBinDir = Get-ChildItem "$env:APPDATA\drako\ReoLauncher-dev\metadata\java" -Recurse -Filter "javap.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $jdkBinDir) { throw "javap not found" }
$javap = $jdkBinDir.FullName
"javap: $javap"
$classes = @(
  'org.polyfrost.polyplus.client.features.JvmAdvisor',
  'org.polyfrost.polyplus.client.features.ModpackDiff',
  'org.polyfrost.polyplus.client.gui.PolyPlusMainMenuScreenKt',
  'org.polyfrost.polyplus.client.gui.PolyPlusOnboardingScreenKt',
  'org.polyfrost.polyplus.client.gui.RamGuideScreenKt',
  'org.polyfrost.polyplus.client.launcher.MicrosoftAuth',
  'org.polyfrost.polyplus.client.launcher.MicrosoftAuth$RedirectPage',
  'org.polyfrost.polyplus.client.PolyPlusConfig',
  'org.polyfrost.polyplus.client.PolyPlusCosmeticsConfig',
  'org.polyfrost.polyplus.client.PolyPlusMainMenuConfig'
)
$fail = 0
foreach ($c in $classes) {
  $null = & $javap -p -cp $outJar $c 2>&1
  if ($LASTEXITCODE -ne 0) { "FAIL: $c"; $fail++ } else { "ok: $c" }
}
if ($fail -gt 0) { throw "$fail classes failed javap validation" }
"--- REOCLIENT check in main menu class ---"
$out = & $javap -v -p -cp $outJar org.polyfrost.polyplus.client.gui.PolyPlusMainMenuScreenKt 2>&1
if ($out -match 'REOCLIENT') { "REOCLIENT present in constant pool" } else { throw "REOCLIENT missing" }
if ($out -match '"ONECLIENT"') { throw "ONECLIENT still present" } else { "ONECLIENT gone" }

# ---------- 5. swap into cluster ----------
if ($Swap) {
  $target = "C:\Users\drako\AppData\Roaming\drako\ReoLauncher-dev\clusters\26.3 Fabric\mods\polyplus-1.3.10+26.3.jar"
  $origHash = (Get-FileHash -Algorithm SHA1 -LiteralPath $target).Hash.ToLower()
  "target hash before: $origHash"
  Remove-Item -LiteralPath $target -Force
  Copy-Item $outJar $target -Force
  $newHash = (Get-FileHash -Algorithm SHA1 -LiteralPath $target).Hash.ToLower()
  "target hash after:  $newHash"
  "hardlinks now:"
  cmd /c "fsutil hardlink list `"$target`""
}
"done"