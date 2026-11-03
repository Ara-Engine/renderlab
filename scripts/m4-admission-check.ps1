param(
 [ValidateSet('build','run','timing','controls','release-observations','images','progress','checks','focused','restore')][string]$Phase,
 [string]$Evidence='.refs/m4-admission',
 [string]$OutputRoot='measurements/m4-stream/admission',
 [ValidatePattern('^[a-zA-Z0-9-]*$')][string]$RunPrefix='',
 [string]$Name,
 [string]$Exe,
 [string]$SourceRecord,
 [string]$Features='',
 [switch]$Release,
 [Parameter(ValueFromRemainingArguments=$true)][string[]]$ReplayArgs
)
$ErrorActionPreference='Stop'
Set-Location (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force -Path "$Evidence/bin" | Out-Null
function Guard {
 if(Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue){throw 'game active; defer heavy/GPU work'}
 if(Test-Path D:/AraBuild/render-v3/quiet-window.lock){throw 'quiet window owned; defer heavy/GPU work'}
 if(Get-Process -Name cargo,rustc,virtual-stream -ErrorAction SilentlyContinue){throw 'competing build/replay process; serialize work'}
}
function Sources {
 $inputs=@('Cargo.toml','Cargo.lock','.cargo/config.toml')+@(Get-ChildItem src,examples,crates/asset,shaders,support -Recurse -File | Select-Object -ExpandProperty FullName)
 @(Get-FileHash $inputs -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash)
}
function Cargo([string]$Label,[string[]]$Arguments) {
 Guard
 if(Test-Path "$Evidence/$Label.process.json"){throw 'fresh process record required'}
 $before=Sources
 $before | ConvertTo-Json | Set-Content "$Evidence/$Label.sources.json"
 $p=[Diagnostics.Process]::new()
 $p.StartInfo.FileName='cargo.exe'
 $p.StartInfo.Arguments=($Arguments|ForEach-Object{'"'+$_.Replace('"','\"')+'"'}) -join ' '
 $p.StartInfo.UseShellExecute=$false
 $p.StartInfo.CreateNoWindow=$true
 $p.StartInfo.RedirectStandardOutput=$true
 $p.StartInfo.RedirectStandardError=$true
 $watch=[Diagnostics.Stopwatch]::StartNew()
 try {
  $null=$p.Start()
  $stdout=$p.StandardOutput.ReadToEndAsync()
  $stderr=$p.StandardError.ReadToEndAsync()
  if(!$p.WaitForExit(1800000)){$p.Kill();$p.WaitForExit();throw 'finite build/check deadline'}
  [IO.File]::WriteAllText("$Evidence/$Label.stdout.log",$stdout.Result)
  [IO.File]::WriteAllText("$Evidence/$Label.stderr.log",$stderr.Result)
  @{args=$Arguments;exit=$p.ExitCode;seconds=$watch.Elapsed.TotalSeconds}|ConvertTo-Json|Set-Content "$Evidence/$Label.process.json"
  if($p.ExitCode -ne 0){throw "$Label failed; retained logs"}
 } finally {$p.Dispose()}
 if(Compare-Object $before (Sources) -Property Path,Hash){throw 'source changed during build/check'}
 Write-Output "$Label passed"
}
function Build([string]$Label,[bool]$Optimized,[string]$Enabled) {
 if(!$Label){throw 'build name required'}
 if(Test-Path "$Evidence/bin/$Label.exe"){throw 'preserved executable exists'}
 $a=@('build','--locked','-p','renderlab','--example','virtual-stream','--no-default-features','-j2')
 $profile='debug'
 if($Optimized){$a+='--release';$profile='release'}
 if($Enabled){$a+=@('--features',$Enabled)}
 Cargo "build-$Label" $a
 Copy-Item "target/$profile/examples/virtual-stream.exe" "$Evidence/bin/$Label.exe"
 Copy-Item "$Evidence/build-$Label.sources.json" "$Evidence/bin/$Label.sources.json"
 @{profile=$profile;features=$Enabled;exe_hash=(Get-FileHash "$Evidence/bin/$Label.exe").Hash;source_record_sha256=(Get-FileHash "$Evidence/bin/$Label.sources.json").Hash;commit=(& git rev-parse HEAD)}|ConvertTo-Json|Set-Content "$Evidence/bin/$Label.json"
}
function Replay([string]$Label,[string]$Binary,[string]$Inventory,[string[]]$Arguments,[bool]$Validate=$true) {
 Guard
 $Label=$RunPrefix+$Label
 if(!$Label -or !$Binary -or !$Inventory){throw 'explicit run name, binary and compiled source inventory required'}
 $run=Join-Path $OutputRoot $Label
 if(Test-Path $run){throw 'fresh output required'}
 Get-Content $Inventory -Raw | ConvertFrom-Json | Out-Null
 $a=@('-NoProfile','-File','scripts/m4-progress-run.ps1','-Name',$Label,'-Exe',$Binary,'-OutputRoot',$OutputRoot)
 if($Validate){$a+='-Validate'}
 & powershell @a @Arguments
 if($LASTEXITCODE -ne 0){throw "$Label failed; retained replay evidence"}
 Copy-Item $Inventory "$run/compiled-sources.json"
 @{exe=(Resolve-Path $Binary).Path;exe_sha256=(Get-FileHash $Binary).Hash;compiled_source_record=(Resolve-Path $Inventory).Path;compiled_source_record_sha256=(Get-FileHash $Inventory).Hash;worktree_source_record_is_executable_identity=$false}|ConvertTo-Json|Set-Content "$run/identity.json"
}
$base='.refs/m4-progress/bin/progress-final.exe'
$baseSources='.refs/m4-progress/bin/progress-final.sources.json'
$cand="$Evidence/bin/candidate-dev.exe"
$candSources="$Evidence/bin/candidate-dev.sources.json"
$sheet=@('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest')
if($Phase -eq 'build'){Build $Name ([bool]$Release) $Features}
if($Phase -eq 'run'){Replay $Name $Exe $SourceRecord $ReplayArgs}
if($Phase -eq 'timing' -or $Phase -eq 'controls'){
 $cases=@('sheet-paced')
 $count=5
 if($Phase -eq 'controls'){$cases=@('sphere-paced','sheet-unpaced','cold-paced');$count=3}
 foreach($case in $cases){
  foreach($i in 1..$count){
   $sides=@('base','cand')
   if($i%2 -eq 0){$sides=@('cand','base')}
   foreach($side in $sides){
    $binary=$base;$inventory=$baseSources
    if($side -eq 'cand'){$binary=$cand;$inventory=$candSources}
    $a=$sheet+@('--frames','631')
    if($case -eq 'sphere-paced'){$a=@('--manifest','measurements/m4-cook/root-unit-a/closed-sphere.manifest','--frames','631')}
    if($case -eq 'cold-paced'){$a=$sheet+@('--start','270','--hold-pose','--frames','600')}
    if($case -ne 'sheet-unpaced'){$a+=@('--frame-period-ms','16.67')}
    Replay "dev-$side-$case-$i" $binary $inventory $a
   }
  }
 }
}
if($Phase -eq 'images'){
 $captures='0,1,2,3,130,140,150,160,170,180,190,200,210,220,230,269,270,271,630'
 foreach($fixture in @('material-thin','closed-sphere')){
  foreach($pace in @('paced','unpaced')){
   $a=@('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--capture',$captures)
   if($pace -eq 'paced'){$a+=@('--frame-period-ms','16.67')}
   Replay "images-$fixture-$pace" $cand $candSources $a
  }
 }
 Replay 'images-cold270' $cand $candSources ($sheet+@('--start','270','--hold-pose','--frames','600','--capture','270','--frame-period-ms','16.67'))
 foreach($pose in @(210,270)){
  Replay "images-history$pose" $cand $candSources ($sheet+@('--frames',"$($pose+1)",'--settle-final','600','--capture',"$pose",'--frame-period-ms','16.67'))
 }
}
if($Phase -eq 'release-observations'){
 foreach($i in 1..3){
  $sides=@('base','cand')
  if($i%2 -eq 0){$sides=@('cand','base')}
  foreach($side in $sides){
   $label='baseline-release'
   if($side -eq 'cand'){$label='candidate-release'}
   Replay "release-$side-sheet-paced-$i" "$Evidence/bin/$label.exe" "$Evidence/bin/$label.sources.json" ($sheet+@('--frames','631','--frame-period-ms','16.67')) $false
  }
 }
}
if($Phase -eq 'progress'){
 foreach($fixture in @('material-thin','closed-sphere')){
  Replay "progress-$fixture-cap1" $cand $candSources @('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--settle-final','10','--candidates','1','--page-log','--frame-period-ms','16.67')
 }
 Replay 'progress-held210-cap1' $cand $candSources ($sheet+@('--frames','211','--settle-final','600','--candidates','1','--page-log','--frame-period-ms','16.67'))
 Replay 'progress-cold270-cap1' $cand $candSources ($sheet+@('--start','270','--hold-pose','--frames','1000','--candidates','1','--page-log','--frame-period-ms','16.67'))
}
if($Phase -eq 'checks'){
 Cargo 'check-all-features' @('check','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2')
 Cargo 'clippy-all-features' @('clippy','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2','--','-D','warnings')
 Cargo 'cpu-all-features' @('test','--locked','-p','renderlab','-p','renderlab-asset','--all-features','-j2','--','--test-threads=1','--nocapture')
}
if($Phase -eq 'focused'){
 $env:VK_IMPLICIT_LAYER_PATH=(Join-Path (Get-Location) 'measurements/empty-vulkan-implicit')
 Cargo $Name @('test','--locked','-p','renderlab','-j2','gpu_virtual_admission_accounting','--','--ignored','--nocapture')
}
if($Phase -eq 'restore'){Build 'restored-normal' $false ''}
