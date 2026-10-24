# Run the four ReoMods all-version build partitions in parallel.
$ErrorActionPreference = 'Stop'
$toolkit = "C:\Users\drako\Documents\Default Project\ReoLauncher\tools\reo-client-branding"
$build = Join-Path $toolkit 'scripts\build_reomods.ps1'
$manifests = Join-Path $toolkit 'manifests'
$hosting = "C:\Users\drako\Documents\Default Project\ReoHosting"
$workRoot = "C:\Users\drako\AppData\Local\Temp\opencode\reomods_all_work"
New-Item -ItemType Directory -Force -Path "$hosting\_buildlogs", $workRoot | Out-Null

$procs = @()
for ($p = 1; $p -le 4; $p++) {
  $tasks = Join-Path $manifests "reomods-all-versions.part-$p.json"
  if (-not (Test-Path -LiteralPath $tasks)) { throw "missing $tasks" }
  $log = Join-Path $hosting "_buildlogs\part-$p.log"
  $err = Join-Path $hosting "_buildlogs\part-$p.err.log"
  $argList = @(
    '-NoProfile',
    '-ExecutionPolicy', 'Bypass',
    '-File', ('"' + $build + '"'),
    '-TasksPath', ('"' + $tasks + '"'),
    '-OutDir', ('"' + (Join-Path $hosting 'mods') + '"'),
    '-OriginalsDir', ('"' + (Join-Path $hosting '_originals') + '"'),
    '-WorkDir', ('"' + (Join-Path $workRoot "part-$p") + '"')
  )
  $proc = Start-Process -FilePath 'powershell.exe' -ArgumentList $argList -RedirectStandardOutput $log -RedirectStandardError $err -PassThru -WindowStyle Hidden
  $procs += [pscustomobject]@{ Part = $p; Proc = $proc; Log = $log; Err = $err }
}
$procs | ForEach-Object { "started part $($_.Part) pid $($_.Proc.Id)" }
$procs | ForEach-Object { $_.Proc.WaitForExit() }
$summary = @()
foreach ($x in $procs) {
  $summary += "part $($x.Part): exit $($x.Proc.ExitCode)  log=$($x.Log)"
}
$summary | Set-Content -LiteralPath (Join-Path $hosting '_buildlogs\summary.txt') -Encoding UTF8
$summary
