param([ValidateSet('build','motion','images','held','timing','checks','restore')][string]$Phase, [string]$Evidence='.refs/m4-progress', [string]$OutputRoot='measurements/m4-stream/progress', [ValidatePattern('^[a-zA-Z0-9-]*$')][string]$RunPrefix='final-')
$ErrorActionPreference='Stop'
Set-Location 'D:/Coding/projects/desktop/renderlab'
$refs=$Evidence
New-Item -ItemType Directory -Force -Path $refs | Out-Null
function Guard {
 if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) {throw 'game active; defer heavy/GPU work'}
 if (Test-Path D:/AraBuild/render-v3/quiet-window.lock) {throw 'quiet window owned'}
}
function Cargo([string]$Name,[string[]]$Arguments) {
 Guard
 $path="$refs/$Name"
 if(Test-Path "$path.process.json"){throw 'fresh process record required'}
 $p=[Diagnostics.Process]::new()
 $p.StartInfo.FileName='cargo.exe'
 $p.StartInfo.Arguments=($Arguments|ForEach-Object{'"'+$_.Replace('"','\"')+'"'}) -join ' '
 $p.StartInfo.UseShellExecute=$false
 $p.StartInfo.CreateNoWindow=$true
 $p.StartInfo.RedirectStandardOutput=$true
 $p.StartInfo.RedirectStandardError=$true
 $p.StartInfo.EnvironmentVariables['VK_IMPLICIT_LAYER_PATH']=(Join-Path (Get-Location) 'measurements/empty-vulkan-implicit')
 $clock=[Diagnostics.Stopwatch]::StartNew()
 try {
  $null=$p.Start();$stdout=$p.StandardOutput.ReadToEndAsync();$stderr=$p.StandardError.ReadToEndAsync()
  if(!$p.WaitForExit(1800000)){$p.Kill();$p.WaitForExit();throw 'finite task deadline'}
  [IO.File]::WriteAllText("$path.stdout.log",$stdout.Result)
  [IO.File]::WriteAllText("$path.stderr.log",$stderr.Result)
  @{args=$Arguments;exit=$p.ExitCode;seconds=$clock.Elapsed.TotalSeconds}|ConvertTo-Json|Set-Content "$path.process.json"
  if($p.ExitCode -ne 0){throw "$Name failed; retained logs"}
 } finally {$p.Dispose()}
 Write-Output "$Name passed"
}
function Sources {
 $inputs=@('Cargo.toml','Cargo.lock','.cargo/config.toml')+@(Get-ChildItem src,examples,crates/asset,shaders,support -Recurse -File | Select-Object -ExpandProperty FullName)
 @(Get-FileHash $inputs -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash)
}
function Build([string]$Name,[string]$Profile,[string]$Features) {
 $before=Sources
 $a=@('build','--locked','-p','renderlab','--example','virtual-stream','--no-default-features','-j2')
 if($Profile -eq 'release'){$a+='--release'}
 if($Features){$a+=@('--features',$Features)}
 Cargo "build-$Name" $a
 if(Compare-Object $before (Sources) -Property Path,Hash){throw 'source changed during build'}
 New-Item -ItemType Directory -Force -Path "$refs/bin" | Out-Null
 if(Test-Path "$refs/bin/$Name.exe"){throw 'preserved executable exists'}
 Copy-Item "target/$Profile/examples/virtual-stream.exe" "$refs/bin/$Name.exe"
 $before | ConvertTo-Json | Set-Content "$refs/bin/$Name.sources.json"
 @{profile=$Profile;features=$Features;exe_hash=(Get-FileHash "$refs/bin/$Name.exe").Hash;commit=(& git rev-parse HEAD);diff=(& git diff --stat);source_record_sha256=(Get-FileHash "$refs/bin/$Name.sources.json").Hash}|ConvertTo-Json|Set-Content "$refs/bin/$Name.json"
}
function Replay([string]$Name,[string]$Exe,[bool]$Validate,[string[]]$Arguments) {
 $Name=$RunPrefix+$Name
 Guard
 $a=@('-NoProfile','-File','scripts/m4-progress-run.ps1','-Name',$Name,'-Exe',$Exe,'-OutputRoot',$OutputRoot)
 if($Validate){$a+='-Validate'}
 $p=[Diagnostics.Process]::new()
 $p.StartInfo.FileName='powershell.exe'
 $p.StartInfo.Arguments=($a+$Arguments|ForEach-Object{'"'+$_.Replace('"','\"')+'"'}) -join ' '
 $p.StartInfo.UseShellExecute=$false
 $p.StartInfo.CreateNoWindow=$true
 $p.StartInfo.RedirectStandardOutput=$true
 $p.StartInfo.RedirectStandardError=$true
 try {
  $null=$p.Start();$stdout=$p.StandardOutput.ReadToEndAsync();$stderr=$p.StandardError.ReadToEndAsync()
  if(!$p.WaitForExit(3600000)){$p.Kill();$p.WaitForExit();throw 'finite replay deadline'}
  [IO.File]::WriteAllText("$refs/$Name.log",$stdout.Result+$stderr.Result)
  if($p.ExitCode -ne 0){throw "$Name failed; retained logs"}
 } finally {$p.Dispose()}
 Write-Output "$Name passed"
}

$exe="$refs/bin/progress-final.exe"
$sheet=@('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest')
if($Phase -eq 'build'){Build 'progress-final' 'debug' ''}
if($Phase -eq 'motion'){
 foreach($fixture in @('material-thin','closed-sphere')){
  foreach($cap in @(1,64,1024)){
   foreach($pace in @('paced','unpaced')){
    $a=@('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--candidates',"$cap",'--page-log')
    if($pace -eq 'paced'){$a+=@('--frame-period-ms','16.67')}
    Replay "motion-$fixture-$cap-$pace" $exe $true $a
   }
  }
 }
}
if($Phase -eq 'held'){
 foreach($cap in @(1,64,1024)){
  Replay "held210-$cap" $exe $true ($sheet+@('--frames','211','--candidates',"$cap",'--settle-final','600','--page-log','--frame-period-ms','16.67'))
  Replay "cold270-$cap" $exe $true ($sheet+@('--start','270','--hold-pose','--frames','1000','--candidates',"$cap",'--page-log','--frame-period-ms','16.67'))
 }
}
if($Phase -eq 'images'){
 $captures='0,1,2,3,130,140,150,160,170,180,190,200,210,220,230,269,270,271,630'
 foreach($fixture in @('material-thin','closed-sphere')){
  foreach($pace in @('paced','unpaced')){
   $a=@('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--capture',$captures)
   if($pace -eq 'paced'){$a+=@('--frame-period-ms','16.67')}
   Replay "images-$fixture-$pace" $exe $true $a
  }
 }
 Replay 'images-cold270' $exe $true ($sheet+@('--start','270','--hold-pose','--frames','600','--capture','270','--frame-period-ms','16.67'))
 foreach($pose in @(210,270)){
  Replay "images-history$pose" $exe $true ($sheet+@('--frames',"$($pose+1)",'--settle-final','600','--capture',"$pose",'--frame-period-ms','16.67'))
 }
}
if($Phase -eq 'timing'){
 foreach($repeat in 1..3){
  Replay "timing-sheet-$repeat" $exe $true ($sheet+@('--frames','631','--frame-period-ms','16.67'))
 }
}
if($Phase -eq 'checks'){
 Cargo 'fmt' @('fmt','--all','--','--check')
 Cargo 'check-all-features' @('check','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2')
 Cargo 'clippy-all-features' @('clippy','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2','--','-D','warnings')
 Cargo 'cpu-all-features' @('test','--locked','-p','renderlab','-p','renderlab-asset','--all-features','-j2','--','--test-threads=1','--nocapture')
}
if($Phase -eq 'restore'){Build 'restored-normal' 'debug' ''}
