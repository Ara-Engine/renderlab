param(
 [ValidateSet('build','run','focused','checks','restore')][string]$Phase,
 [string]$Evidence='.refs/m4-startup',
 [string]$OutputRoot='measurements/m4-stream/startup',
 [string]$Name,
 [string]$Exe,
 [string]$SourceRecord,
 [Parameter(ValueFromRemainingArguments=$true)][string[]]$ReplayArgs
)
$ErrorActionPreference='Stop'
Set-Location (Resolve-Path "$PSScriptRoot/..").Path
New-Item -ItemType Directory -Force -Path "$Evidence/bin" | Out-Null
function Guard {
 if(Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue){throw 'game active; GPU/heavy work deferred'}
 if(Test-Path D:/AraBuild/render-v3/quiet-window.lock){throw 'quiet window owned; GPU/heavy work deferred'}
 if(Get-Process -Name cargo,rustc,virtual-stream -ErrorAction SilentlyContinue){throw 'competing task; serialize build/GPU work'}
}
function Cargo([string]$Label,[string[]]$Arguments) {
 Guard
 if(Test-Path "$Evidence/$Label.process.json"){throw 'fresh name required'}
 $inputs=@('Cargo.toml','Cargo.lock','.cargo/config.toml','viewer/Cargo.toml')+@(Get-ChildItem src,viewer/src,examples,crates/asset,shaders,support -Recurse -File | Select-Object -ExpandProperty FullName)
 $before=@(Get-FileHash $inputs | Sort-Object Path | Select-Object Path,Hash)
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
  $stdout=$p.StandardOutput.ReadToEndAsync(); $stderr=$p.StandardError.ReadToEndAsync()
  if(!$p.WaitForExit(600000)){$p.Kill();$p.WaitForExit();throw 'finite Cargo deadline'}
  [IO.File]::WriteAllText("$Evidence/$Label.stdout.log",$stdout.Result)
  [IO.File]::WriteAllText("$Evidence/$Label.stderr.log",$stderr.Result)
  @{args=$Arguments;exit=$p.ExitCode;seconds=$watch.Elapsed.TotalSeconds}|ConvertTo-Json|Set-Content "$Evidence/$Label.process.json"
  if($p.ExitCode -ne 0){throw "$Label failed; logs retained"}
 } finally {$p.Dispose()}
 if(Compare-Object $before @(Get-FileHash $inputs | Sort-Object Path | Select-Object Path,Hash) -Property Path,Hash){throw 'source changed during Cargo'}
 Write-Output "$Label passed"
}
if($Phase -eq 'focused'){
 $env:VK_IMPLICIT_LAYER_PATH=(Join-Path (Get-Location) 'measurements/empty-vulkan-implicit')
 Cargo $Name @('test','--locked','-p','renderlab','-j2','gpu_virtual_first_view','--','--ignored','--nocapture','--test-threads=1')
}
if($Phase -eq 'build' -or $Phase -eq 'restore'){
 if(!$Name){throw 'build name required'}
 if(Test-Path "$Evidence/bin/$Name.exe"){throw 'preserved executable exists'}
 Cargo "build-$Name" @('build','--locked','-p','renderlab','--example','virtual-stream','--no-default-features','-j2')
 Copy-Item target/debug/examples/virtual-stream.exe "$Evidence/bin/$Name.exe"
 Copy-Item "$Evidence/build-$Name.sources.json" "$Evidence/bin/$Name.sources.json"
 Cargo "viewer-$Name" @('build','--locked','-p','renderlab-viewer','--no-default-features','-j2')
 Copy-Item target/debug/renderlab-viewer.exe "$Evidence/bin/$Name-viewer.exe"
 Get-FileHash "$Evidence/bin/$Name*" | ConvertTo-Json | Set-Content "$Evidence/$Name-identities.json"
}
if($Phase -eq 'run'){
 Guard
 if(!$Exe){$Exe="$Evidence/bin/candidate-dev.exe"}
 if(!$SourceRecord){$SourceRecord="$Evidence/bin/candidate-dev.sources.json"}
 & powershell -NoProfile -File scripts/m4-admission-check.ps1 -Phase run -Evidence $Evidence -OutputRoot $OutputRoot -Name $Name -Exe $Exe -SourceRecord $SourceRecord @ReplayArgs
 if($LASTEXITCODE -ne 0){throw 'replay failed; evidence retained'}
}
if($Phase -eq 'checks'){
 Guard
 & powershell -NoProfile -File scripts/m4-runtime-check.ps1 -Gpu -Output "$OutputRoot/$Name"
 if($LASTEXITCODE -ne 0){throw 'runtime checks failed'}
 Cargo "$Name-check-all-features" @('check','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2')
 Cargo "$Name-clippy-all-features" @('clippy','--locked','-p','renderlab','-p','renderlab-asset','--all-targets','--all-features','-j2','--','-D','warnings')
 Cargo "$Name-cpu-all-features" @('test','--locked','-p','renderlab','-p','renderlab-asset','--all-features','-j2','--','--test-threads=1')
}
