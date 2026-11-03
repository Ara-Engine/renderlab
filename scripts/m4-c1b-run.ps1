param(
    [Parameter(Mandatory=$true)][string]$Name,
    [string]$Exe='target/debug/examples/virtual-stream.exe',
    [switch]$Validate,
    [Parameter(ValueFromRemainingArguments=$true)][string[]]$ReplayArgs
)
$ErrorActionPreference='Stop'
$root=(Resolve-Path "$PSScriptRoot/..").Path
Set-Location $root
if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) {throw 'game active; defer measurement'}
if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') {throw 'quiet window owned; defer measurement'}
$run=Join-Path $root "measurements/m4-stream/c1b/$Name"
if (Test-Path $run) {throw 'fresh output required; preserve all prior evidence'}
New-Item -ItemType Directory -Path $run | Out-Null
$inputs=@('Cargo.toml','Cargo.lock','.cargo/config.toml')+@(Get-ChildItem src,examples,crates/asset,shaders,support -Recurse -File | Select-Object -ExpandProperty FullName)
$before=@(Get-FileHash $inputs -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash)
$before | ConvertTo-Json | Set-Content "$run/source-hashes.json"
Get-FileHash $Exe -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/executable.json"
Get-FileHash measurements/m4-cook/registration-initial/cameras.json,measurements/m4-cook/root-unit-a/*.manifest,measurements/m4-cook/root-unit-a/cache/virtual-v1/packages/* | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/content.json"
git diff | Set-Content "$run/source.diff"
Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | ConvertTo-Json | Set-Content "$run/cpu.json"
Get-Process | Sort-Object CPU -Descending | Select-Object -First 15 Name,Id,CPU,WorkingSet64 | ConvertTo-Json | Set-Content "$run/processes.json"
& nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-before.csv"
$p=[Diagnostics.Process]::new()
$p.StartInfo.FileName=(Resolve-Path $Exe).Path
$argv=@($ReplayArgs)+@('--output',"$run/replay")
if ($Validate) {$argv=@('--validate')+$argv}
$p.StartInfo.Arguments=($argv|ForEach-Object{'"'+$_.Replace('"','\"')+'"'}) -join ' '
$p.StartInfo.UseShellExecute=$false
$p.StartInfo.CreateNoWindow=$true
$p.StartInfo.RedirectStandardOutput=$true
$p.StartInfo.RedirectStandardError=$true
$p.StartInfo.EnvironmentVariables['VK_IMPLICIT_LAYER_PATH']=(Join-Path $root 'measurements/empty-vulkan-implicit')
$watch=[Diagnostics.Stopwatch]::StartNew()
try {
    $null=$p.Start()
    $stdout=$p.StandardOutput.ReadToEndAsync()
    $stderr=$p.StandardError.ReadToEndAsync()
    if (!$p.WaitForExit(1800000)) {$p.Kill();$p.WaitForExit();throw 'finite 30min measurement deadline'}
    [IO.File]::WriteAllText("$run/stdout.log",$stdout.Result)
    [IO.File]::WriteAllText("$run/stderr.log",$stderr.Result)
    @{args=$argv;exit=$p.ExitCode;seconds=$watch.Elapsed.TotalSeconds;cpu_seconds=$p.TotalProcessorTime.TotalSeconds;validate=[bool]$Validate}|ConvertTo-Json|Set-Content "$run/process.json"
    if ($p.ExitCode -ne 0) {throw 'replay failed; retained evidence'}
    if ($Validate -and $stderr.Result -notmatch 'gpu_validation_summary errors=0 missing_layer=0 active_layer=1') {throw 'clean active validation not observed'}
} finally {$p.Dispose()}
$after=@(Get-FileHash $inputs -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash)
$after | ConvertTo-Json | Set-Content "$run/source-hashes-after.json"
if (Compare-Object $before $after -Property Path,Hash) {throw 'source changed during run; mixed identity'}
Write-Output "$Name passed execution and requested validation"

