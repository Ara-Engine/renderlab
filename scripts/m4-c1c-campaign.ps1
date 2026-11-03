param([ValidateSet('base','build','checks','pairs','release','images','cpu','dense','shadow','profile','restore')][string]$Phase)
$ErrorActionPreference='Stop'
Set-Location 'D:/Coding/projects/desktop/renderlab'
$refs='.refs/m4-c1c'
function Guard {
 if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) {throw 'game active; defer heavy/GPU work'}
 if (Test-Path D:/AraBuild/render-v3/quiet-window.lock) {throw 'quiet window owned'}
}
function Cargo([string]$Name,[string[]]$Arguments,[string]$Evidence='') {
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
 if($Evidence){$p.StartInfo.EnvironmentVariables['RENDERLAB_EVIDENCE_DIR']=(Join-Path (Get-Location) $Evidence)}
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
 Guard
 $a=@('-NoProfile','-File','scripts/m4-c1c-run.ps1','-Name',$Name,'-Exe',$Exe)
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
function Fixture([string]$Fixture,[string]$Pace) {
 $a=@('--frames','631')
 if($Fixture -eq 'cold'){$a=@('--start','270','--hold-pose','--frames','600','--capture','270')}
 if($Fixture -ne 'sphere'){$a+=@('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest')}
 if($Pace -eq 'paced'){$a+=@('--frame-period-ms','16.67')}
 $a
}
# Baseline executables are the preserved C1b bytes; their source record is the C1b record
# 18ebbb19..., not the worktree snapshot each helper run stores.
function Pairs([string]$Profile,[bool]$Validate,[string[]]$Configs) {
 $exes=@{base=".refs/m4-c1b/bin/final-$Profile.exe";cand="$refs/bin/cand-$Profile.exe"}
 foreach($repeat in 1..5){
  $order=if($repeat % 2){@('base','cand')}else{@('cand','base')}
  foreach($config in $Configs){
   $fixture,$pace=$config.Split('-')
   foreach($side in $order){Replay "$Profile-$side-$fixture-$pace-$repeat" $exes[$side] $Validate (Fixture $fixture $pace)}
  }
 }
}
$captures='0,1,2,3,4,5,60,70,80,90,100,110,120,130,140,150,160,170,180,190,200,210,220,230,240,250,260,269,270,271,280,290,300,310,320,330,630'
if($Phase -eq 'base') {
 $record=Get-Content measurements/m4-stream/c1b/final-dev-sheet-paced-1/source-hashes.json -Raw | ConvertFrom-Json
 if(Compare-Object @($record) (Sources) -Property Path,Hash){throw 'worktree differs from C1b measured source; baseline build would be mislabeled'}
 Build 'base-release-policy' 'release' 'selection-oracle'
}
if($Phase -eq 'build') {
 Build 'cand-dev' 'debug' ''
 Build 'cand-release' 'release' ''
 Build 'cand-policy' 'debug' 'selection-oracle'
 Build 'cand-release-policy' 'release' 'selection-oracle'
 Build 'cand-profile' 'debug' 'selection-profile'
 Build 'cand-shadow' 'debug' 'selection-shadow'
}
if($Phase -eq 'checks') {
 Cargo 'fmt' @('fmt','--all','--','--check')
 Cargo 'check' @('check','--locked','--workspace','--all-targets','-j2')
 Cargo 'check-all-features' @('check','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2')
 Cargo 'clippy' @('clippy','--locked','--workspace','--all-targets','-j2','--','-D','warnings')
 Cargo 'clippy-all-features' @('clippy','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2','--','-D','warnings')
 Cargo 'cpu-all-features' @('test','--locked','-p','renderlab','-p','renderlab-asset','--all-features','-j2','--','--test-threads=1','--nocapture')
}
if($Phase -eq 'pairs'){Pairs 'dev' $true @('sheet-paced','sphere-paced','sheet-unpaced','sphere-unpaced','cold-paced')}
if($Phase -eq 'release'){Pairs 'release' $false @('sheet-paced','sphere-paced')}
if($Phase -eq 'images') {
 $exe="$refs/bin/cand-dev.exe"
 foreach($fixture in @('material-thin','closed-sphere')) {
  foreach($pace in @('paced','unpaced')) {
   $a=@('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--capture',$captures)
   if($pace -eq 'paced'){$a+=@('--frame-period-ms','16.67')}
   Replay "images-$fixture-$pace" $exe $true $a
  }
 }
 foreach($pose in @('210','270')) {
  $frames=1+[int]$pose
  Replay "history-$pose" $exe $true @('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest','--frames',"$frames",'--settle-final','600','--capture',$pose,'--frame-period-ms','16.67','--page-log')
 }
 Replay 'p1-paced' $exe $true @('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest','--frames','211','--capture','210','--page-log','--frame-period-ms','16.67')
}
if($Phase -eq 'cpu') {
 $exes=@{base='.refs/m4-c1b/bin/final-policy.exe';cand="$refs/bin/cand-policy.exe";'base-release'="$refs/bin/base-release-policy.exe";'cand-release'="$refs/bin/cand-release-policy.exe"}
 foreach($profile in @('dev','release')){
  foreach($repeat in 1..2){
   foreach($fixture in @('material-thin','closed-sphere')){
    $sides=if($repeat % 2){@('base','cand')}else{@('cand','base')}
    foreach($side in $sides){
     $key=if($profile -eq 'dev'){$side}else{"$side-release"}
     Replay "cpu-$profile-$side-$fixture-$repeat" $exes[$key] $false @('--mode','cpu','--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--cpu-stride','30','--repeats','5')
    }
   }
  }
 }
}
if($Phase -eq 'dense') {
 foreach($fixture in @('material-thin','closed-sphere')) {
  Replay "dense-$fixture" "$refs/bin/cand-policy.exe" $false @('--mode','cpu','--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--cpu-stride','1','--repeats','1')
 }
}
if($Phase -eq 'shadow') {
 foreach($fixture in @('material-thin','closed-sphere')) {
  Replay "shadow-$fixture" "$refs/bin/cand-shadow.exe" $true @('--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--frames','631','--capture','210,270,630','--frame-period-ms','16.67')
 }
 Replay 'shadow-cold' "$refs/bin/cand-shadow.exe" $true @('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest','--start','270','--hold-pose','--frames','600','--capture','270','--frame-period-ms','16.67')
}
if($Phase -eq 'profile') {
 foreach($repeat in 1..2){
  $sides=if($repeat % 2){@('base','cand')}else{@('cand','base')}
  foreach($side in $sides){
   $exe=if($side -eq 'base'){'.refs/m4-c1b/bin/final-profile.exe'}else{"$refs/bin/cand-profile.exe"}
   Replay "profile-$side-sheet-$repeat" $exe $true @('--manifest','measurements/m4-cook/root-unit-a/material-thin.manifest','--frames','631','--frame-period-ms','16.67')
  }
 }
}
if($Phase -eq 'restore') {
 Cargo 'restore-default' @('build','--locked','-p','renderlab','--example','virtual-stream','--no-default-features','-j2')
 $ErrorActionPreference='Continue'
 & target/debug/examples/virtual-stream.exe --mode cpu --frames 1 --output measurements/m4-stream/c1c/default-feature-probe *> "$refs/default-feature-probe.log"
 $code=$LASTEXITCODE
 @{exit=$code;expected_exit=1;exe_hash=(Get-FileHash target/debug/examples/virtual-stream.exe).Hash}|ConvertTo-Json|Set-Content "$refs/default-feature-probe.json"
 if($code -ne 1 -or (Get-Content "$refs/default-feature-probe.log" -Raw) -notmatch 'requires --features selection-oracle'){throw 'normal feature probe did not reject CPU oracle mode'}
}
