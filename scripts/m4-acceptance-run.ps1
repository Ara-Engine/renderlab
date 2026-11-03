param(
 [Parameter(Mandatory=$true)][ValidateSet('build-dev','build-release','capacity','rejections','images','timing','guard-diagnostic','checks','restore','probe')][string]$Phase,
 [string]$Evidence='.refs/m4-acceptance',
 [string]$OutputRoot='measurements/m4-acceptance'
)
$ErrorActionPreference='Stop'
Set-Location (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force "$Evidence/bin",$OutputRoot | Out-Null
function Guard {
 if(Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue){throw 'game active; defer heavy/GPU work'}
 if(Test-Path D:/AraBuild/render-v3/quiet-window.lock){throw 'quiet-window lock active; defer heavy/GPU work'}
 if(Get-Process -Name cargo,rustc,virtual-stream,acceptance-dev,acceptance-release -ErrorAction SilentlyContinue){throw 'competing build/replay; serialize work'}
}
function Sources {
 $files=@('Cargo.toml','Cargo.lock','.cargo/config.toml','viewer/Cargo.toml')+@(Get-ChildItem src,examples,crates/asset,shaders,support,viewer/src -Recurse -File | Select-Object -ExpandProperty FullName)
 @(Get-FileHash $files | Sort-Object Path | Select-Object Path,Hash)
}
function Run([string]$Label,[string]$Binary,[string[]]$Arguments,[string]$Directory,[int]$Expected=0) {
 Guard
 if(Test-Path "$Directory/process.json"){throw 'fresh process label required; preserve evidence'}
 New-Item -ItemType Directory -Force $Directory | Out-Null
 $before=Sources
 $before | ConvertTo-Json | Set-Content "$Directory/sources-before.json"
 $p=[Diagnostics.Process]::new()
 $p.StartInfo.FileName=$Binary
 $p.StartInfo.Arguments=($Arguments|ForEach-Object{'"'+$_.Replace('"','\"')+'"'}) -join ' '
 $p.StartInfo.UseShellExecute=$false
 $p.StartInfo.CreateNoWindow=$true
 $p.StartInfo.RedirectStandardOutput=$true
 $p.StartInfo.RedirectStandardError=$true
 $p.StartInfo.EnvironmentVariables['VK_IMPLICIT_LAYER_PATH']=(Join-Path (Get-Location) 'measurements/empty-vulkan-implicit')
 $watch=[Diagnostics.Stopwatch]::StartNew()
 try {
  $null=$p.Start()
  $stdout=$p.StandardOutput.ReadToEndAsync()
  $stderr=$p.StandardError.ReadToEndAsync()
  if(!$p.WaitForExit(1800000)){$p.Kill();$p.WaitForExit();throw 'finite owned-process deadline; evidence retained'}
  [IO.File]::WriteAllText("$Directory/stdout.log",$stdout.Result)
  [IO.File]::WriteAllText("$Directory/stderr.log",$stderr.Result)
  @{label=$Label;exe=$Binary;args=$Arguments;exit=$p.ExitCode;seconds=$watch.Elapsed.TotalSeconds;cpu_seconds=$p.TotalProcessorTime.TotalSeconds;peak_working_set=$p.PeakWorkingSet64} | ConvertTo-Json -Depth 5 | Set-Content "$Directory/process.json"
  if($p.ExitCode -ne $Expected){throw "$Label unexpected exit; evidence retained"}
  if($Arguments -contains '--validate' -and $Expected -eq 0 -and $stderr.Result -notmatch 'gpu_validation_summary errors=0 missing_layer=0 active_layer=[1-9]'){throw 'clean active validation missing'}
 } finally {$p.Dispose()}
 $after=Sources
 $after | ConvertTo-Json | Set-Content "$Directory/sources-after.json"
 if(Compare-Object $before $after -Property Path,Hash){throw 'source changed during process; mixed identity'}
 Write-Output "$Label passed"
}
function Build([string]$Name,[bool]$Release) {
 $a=@('build','--locked','-p','renderlab','--example','virtual-stream','--no-default-features','-j','2')
 if($Release){$a+='--release'}
 Run "build-$Name" 'cargo.exe' $a "$Evidence/build-$Name"
 $profile='debug';if($Release){$profile='release'}
 Copy-Item "target/$profile/examples/virtual-stream.exe" "$Evidence/bin/$Name.exe"
 Copy-Item "$Evidence/build-$Name/sources-before.json" "$Evidence/bin/$Name.sources.json"
 Get-FileHash "$Evidence/bin/$Name*" | ConvertTo-Json | Set-Content "$Evidence/$Name-identities.json"
}
function Replay([string]$Name,[string]$Fixture,[string[]]$Arguments,[bool]$Release=$false,[int]$Expected=0) {
 $binary='acceptance-dev';if($Release){$binary='acceptance-release'}
 $run="$OutputRoot/$Name"
 if(Test-Path $run){throw 'fresh replay output required; preserve all prior runs'}
 New-Item -ItemType Directory $run | Out-Null
 Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | ConvertTo-Json | Set-Content "$run/cpu.json"
 Get-Process | Sort-Object CPU -Descending | Select-Object -First 15 Name,Id,CPU,WorkingSet64 | ConvertTo-Json | Set-Content "$run/processes.json"
 & nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-before.csv"
 Get-FileHash measurements/m4-cook/registration-initial/cameras.json,measurements/m4-cook/root-unit-a/*.manifest,measurements/m4-cook/root-unit-a/cache/virtual-v1/packages/* | ConvertTo-Json | Set-Content "$run/content.json"
 Copy-Item "$Evidence/bin/$binary.sources.json" "$run/compiled-sources.json"
 Get-FileHash "$Evidence/bin/$binary.exe","$Evidence/bin/$binary.sources.json" | ConvertTo-Json | Set-Content "$run/identity.json"
 $a=@('--manifest',"measurements/m4-cook/root-unit-a/$Fixture.manifest",'--limits','.refs/m4-limits/registered-defaults.json','--output',"$run/replay")+$Arguments
 Run $Name (Resolve-Path "$Evidence/bin/$binary.exe").Path $a $run $Expected
 & nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-after.csv"
}
if($Phase -eq 'build-dev'){Build 'acceptance-dev' $false}
if($Phase -eq 'build-release'){Build 'acceptance-release' $true}
if($Phase -eq 'capacity'){
 $cases=@(@('a',1,0,0),@('b',64,64,100),@('c',1024,4096,0),@('d',64,0,100),@('e',1024,64,0),@('f',1,4096,100))
 foreach($fixture in @('material-thin','closed-sphere')){
  $minimum=13;if($fixture -eq 'closed-sphere'){$minimum=1}
  foreach($case in $cases){
   $a=@('--frames','631','--settle-final','300','--frame-period-ms','16.67','--candidates',"$($case[1])",'--output-clusters',"$($minimum+$case[2])",'--io-delay-ms',"$($case[3])",'--validate','--verify-cover','--page-log')
   if($case[2] -eq 0){$a+=@('--capture','0,210,269,270,630')}
   Replay "capacity-$fixture-$($case[0])" $fixture $a
  }
 }
}
if($Phase -eq 'rejections'){
 foreach($fixture in @('material-thin','closed-sphere')){
  $minimum=13;$geometry=196608
  if($fixture -eq 'closed-sphere'){$minimum=1;$geometry=131072}
  Replay "below-output-$fixture" $fixture @('--frames','1','--output-clusters',"$($minimum-1)",'--validate') $false 1
  Replay "below-geometry-$fixture" $fixture @('--frames','1','--geometry-bytes',"$($geometry-1)",'--validate') $false 1
  Replay "minimum-geometry-$fixture" $fixture @('--frames','631','--settle-final','300','--geometry-bytes',"$geometry",'--candidates','64','--validate','--verify-cover','--page-log','--frame-period-ms','16.67')
 }
}
if($Phase -eq 'images'){
 foreach($fixture in @('material-thin','closed-sphere')){
  foreach($mode in @('stream','simple')){
   Replay "equal-$fixture-$mode" $fixture @('--mode',$mode,'--frames','631','--settle-final','300','--frame-period-ms','16.67','--capture','0,210,269,270,630','--validate','--verify-cover') $true
  }
 }
}
if($Phase -eq 'timing'){
 foreach($i in 1..3){
  foreach($fixture in @('material-thin','closed-sphere')){
   Replay "timing-$fixture-$i" $fixture @('--frames','631','--frame-period-ms','16.67') $true
  }
 }
}
if($Phase -eq 'checks'){
 Run 'fmt' 'cargo.exe' @('fmt','--all','--','--check') "$Evidence/check-fmt"
 Run 'check-example' 'cargo.exe' @('check','--locked','-p','renderlab','--example','virtual-stream','-j','2') "$Evidence/check-example"
 Run 'clippy-example' 'cargo.exe' @('clippy','--locked','-p','renderlab','--example','virtual-stream','-j','2','--','-D','warnings') "$Evidence/clippy-example"
 Run 'test-example' 'cargo.exe' @('test','--locked','-p','renderlab','--example','virtual-stream','-j','2') "$Evidence/test-example"
}
if($Phase -eq 'guard-diagnostic'){
 foreach($i in 1..3){
  Run "guard-diagnostic-$i" 'powershell.exe' @('-NoProfile','-Command',"if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) { exit 9 }") "$Evidence/guard-diagnostic-$i"
 }
}
if($Phase -eq 'restore'){
 Build 'restored-normal' $false
 Run 'restore-viewer' 'cargo.exe' @('build','--locked','-p','renderlab-viewer','--no-default-features','-j','2') "$Evidence/restore-viewer"
 Get-FileHash target/debug/examples/virtual-stream.exe,target/debug/renderlab-viewer.exe | ConvertTo-Json | Set-Content "$Evidence/restored-binaries.json"
}
if($Phase -eq 'probe'){
 Run 'normal-feature-probe' (Resolve-Path "$Evidence/bin/restored-normal.exe").Path @('--mode','cpu','--frames','1','--output',"$Evidence/normal-feature-probe/replay") "$Evidence/normal-feature-probe" 1
 $record=Get-Content "$Evidence/normal-feature-probe/replay/run.txt" -Raw
 if($record -notmatch 'oracle=false shadow=false profiling=false'){throw 'normal features not restored'}
}
