param([string]$Output = "measurements/m4-cook/check-$(Get-Date -Format yyyyMMdd-HHmmss)", [string]$Registration = 'measurements/m4-cook/registration-initial')
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (Test-Path -LiteralPath $Output) { throw 'Choose a fresh evidence directory.' }
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $Output).Path
function Quiet-Check {
    if (Test-Path D:/AraBuild/render-v3/quiet-window.lock) {
        $lock = Get-Content -Raw D:/AraBuild/render-v3/quiet-window.lock
        if ($lock -match 'expires=(\S+)' -and [DateTimeOffset]::Parse($Matches[1]) -lt [DateTimeOffset]::Now) { return }
        throw "Another job holds the quiet window: $lock"
    }
    if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) { throw 'Foreground gameplay may have returned; defer cooker measurements.' }
}
function Source-Hashes {
    $paths = @('Cargo.toml','Cargo.lock','.cargo/config.toml','scripts/m4-cook-check.ps1') + @(Get-ChildItem crates/asset,crates/cook -Recurse -File | Select-Object -ExpandProperty FullName)
    Get-FileHash -LiteralPath $paths -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash
}
function Cargo-Run([string]$Label,[string[]]$Arguments) {
    Quiet-Check
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $ErrorActionPreference='Continue'
    & cargo @Arguments 2>&1 | ForEach-Object { $_.ToString() } | Set-Content (Join-Path $outputRoot "$Label.log")
    if ($LASTEXITCODE -ne 0) { throw "$Label failed; read $outputRoot/$Label.log" }
    [PSCustomObject]@{ command="cargo $($Arguments -join ' ')"; elapsed_ms=$timer.Elapsed.TotalMilliseconds }
}
$before=@(Source-Hashes)
$before | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'source-hashes.json')
$checks=@()
$checks+=Cargo-Run 'fmt' @('fmt','--all','--','--check')
$checks+=Cargo-Run 'check' @('check','-j','2','--locked','--workspace','--all-targets')
$checks+=Cargo-Run 'clippy' @('clippy','-j','2','--locked','--workspace','--all-targets','--','-D','warnings')
$checks+=Cargo-Run 'tests' @('test','-j','2','--locked','--workspace','--','--nocapture')
$checks | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'checks.json')
foreach ($package in @('renderlab','renderlab-viewer','renderlab-asset','renderlab-cook')) {
    Quiet-Check
    $tree=& cargo tree --locked -p $package --edges normal,build
    if ($LASTEXITCODE -ne 0) { throw "Dependency tree failed: $package" }
    $tree | Set-Content (Join-Path $outputRoot "$package-tree.txt")
    if ($package -ne 'renderlab-cook' -and ($tree -match '\b(meshopt|float-cmp|cc|gltf|image|png|zune-jpeg|sha2) v')) { throw "Cooker dependency leaked into $package" }
}
$checks+=Cargo-Run 'cli-build' @('build','-j','2','--locked','-p','renderlab-cook')
$executable=(Resolve-Path target/debug/renderlab-cook.exe).Path
$cache=Join-Path $outputRoot 'cache'
$hostBefore=Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,TotalVisibleMemorySize,FreePhysicalMemory
function CLI-Run([string]$Label,[string[]]$Arguments,[bool]$ExpectedSuccess=$true) {
    Quiet-Check
    $out=Join-Path $outputRoot "$Label.json"
    $err=Join-Path $outputRoot "$Label.stderr"
    $quoted=@($Arguments | ForEach-Object { if ($_ -match '"') { throw 'Unexpected quote in argument' }; '"'+$_+'"' })
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $process=Start-Process -FilePath $executable -ArgumentList $quoted -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError $err
    # Windows PowerShell may otherwise lose ExitCode while polling a process returned by Start-Process.
    $null=$process.Handle
    $peakWorking=0L; $peakPrivate=0L; $cpuSeconds=0.
    while (-not $process.HasExited) {
        $process.Refresh()
        if (-not $process.HasExited) {
            $peakWorking=[Math]::Max($peakWorking,$process.PeakWorkingSet64)
            $peakPrivate=[Math]::Max($peakPrivate,$process.PrivateMemorySize64)
            $cpuSeconds=$process.TotalProcessorTime.TotalSeconds
        }
        Start-Sleep -Milliseconds 40
    }
    $process.WaitForExit()
    if (($process.ExitCode -eq 0) -ne $ExpectedSuccess) { throw "Unexpected CLI status $($process.ExitCode): $Label; read $err" }
    [PSCustomObject]@{label=$Label;arguments=$Arguments;exit_code=$process.ExitCode;wall_ms=$timer.Elapsed.TotalMilliseconds;sampled_peak_working_set_bytes=$peakWorking;sampled_peak_private_bytes=$peakPrivate;sampled_cpu_seconds=$cpuSeconds}
}
$runs=@()
$registered=Get-Content -Raw (Join-Path $Registration 'registration.json') | ConvertFrom-Json
$runs+=CLI-Run 'regenerated-registration' @('m4-register',(Join-Path $outputRoot 'regenerated'))
$regenerated=Get-Content -Raw (Join-Path $outputRoot 'regenerated/registration.json') | ConvertFrom-Json
if ($regenerated.camera_json_sha256 -ne $registered.camera_json_sha256) { throw 'Deterministic camera regeneration mismatch.' }
foreach ($fixture in $registered.fixtures) {
    $again=$regenerated.fixtures | Where-Object name -eq $fixture.name
    if ($again.prepared_sha256 -ne $fixture.prepared_sha256 -or $again.material_texture_sha256 -ne $fixture.material_texture_sha256) { throw 'Deterministic fixture regeneration mismatch.' }
}
foreach ($fixture in $registered.fixtures) {
    $name=$fixture.name
    $source=Join-Path $Registration $fixture.prepared_file
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -ne $fixture.prepared_sha256) { throw 'Registered full-detail reference changed.' }
    $manifest=Join-Path $outputRoot "$name.manifest"
    $runs+=CLI-Run "$name-cold" @('m4-cook',$source,$cache,$manifest)
    $runs+=CLI-Run "$name-warm" @('m4-cook',$source,$cache,$manifest)
    $cold=Get-Content -Raw (Join-Path $outputRoot "$name-cold.json") | ConvertFrom-Json
    $warm=Get-Content -Raw (Join-Path $outputRoot "$name-warm.json") | ConvertFrom-Json
    if ($cold.cache_hit -or -not $warm.cache_hit -or $cold.package_sha256 -ne $warm.package_sha256) { throw 'Cold/warm identity mismatch.' }
    if ($fixture.full_detail_geometry_bytes -le 4*$cold.counts.normal_pool_bytes) { throw 'Actual root pages make fixture too small for its normal pool.' }
    $kind=if ($name -eq 'closed-sphere') {'closed'} else {'open'}
    $runs+=CLI-Run "$name-proofs" @('m4-prove',$cache,$manifest,$kind)
    & py -3 crates/cook/tools/inspect_virtual.py $cache $manifest > (Join-Path $outputRoot "$name-independent.json")
    if ($LASTEXITCODE -ne 0) { throw 'Independent binary/page inspection failed.' }
    $runs+=CLI-Run "$name-settings-change" @('m4-cook',$source,$cache,(Join-Path $outputRoot "$name-settings.manifest"),'--triangles64')
    $changed=Join-Path $outputRoot "$name-changed.rla"
    & py -3 crates/cook/tools/inspect_virtual.py change-source $source $changed > (Join-Path $outputRoot "$name-changed-source.json")
    if ($LASTEXITCODE -ne 0) { throw 'Changed-source generator failed.' }
    $runs+=CLI-Run "$name-source-change" @('m4-cook',$changed,$cache,(Join-Path $outputRoot "$name-changed.manifest"))
    foreach ($mode in @('settings-change','source-change')) {
        $changedResult=Get-Content -Raw (Join-Path $outputRoot "$name-$mode.json") | ConvertFrom-Json
        if ($changedResult.cache_hit -or $changedResult.key -eq $cold.key) { throw 'Input/settings invalidation failed.' }
    }
    # A corrupt page must fail inspection and be repaired by recooking, preserving identity.
    $metadataPath=Join-Path $cache "virtual-v1/packages/$($cold.package_sha256)"
    $metadata=[IO.File]::ReadAllBytes($metadataPath)
    $firstPageHash=([BitConverter]::ToString($metadata[($metadata.Length-32)..($metadata.Length-1)])).Replace('-','').ToLowerInvariant()
    $pagePath=Join-Path $cache "virtual-v1/pages/$firstPageHash"
    $page=[IO.File]::ReadAllBytes($pagePath);$page[0]=$page[0] -bxor 1;[IO.File]::WriteAllBytes($pagePath,$page)
    $runs+=CLI-Run "$name-corrupt-rejected" @('m4-inspect',$cache,$manifest) $false
    $runs+=CLI-Run "$name-corrupt-repair" @('m4-cook',$source,$cache,$manifest)
    $repair=Get-Content -Raw (Join-Path $outputRoot "$name-corrupt-repair.json") | ConvertFrom-Json
    if ($repair.cache_hit -or $repair.package_sha256 -ne $cold.package_sha256) { throw 'Corruption repair mismatch.' }
    $accepted=[IO.File]::ReadAllBytes($manifest)
    [IO.File]::WriteAllBytes($manifest,$accepted[0..16])
    $runs+=CLI-Run "$name-partial-rejected" @('m4-inspect',$cache,$manifest) $false
    $runs+=CLI-Run "$name-partial-repair" @('m4-cook',$source,$cache,$manifest)
    if (-not (Get-Content -Raw (Join-Path $outputRoot "$name-partial-repair.json") | ConvertFrom-Json).cache_hit) { throw 'Current-manifest repair should reuse valid key.' }
}
$runs | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $outputRoot 'process-runs.json')
[PSCustomObject]@{timestamp=(Get-Date).ToString('o');head=(& git rev-parse HEAD);rustc=(& rustc -Vv) -join "`n";binary=$executable;binary_sha256=(Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash;cpu=(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors);os_before=$hostBefore;os_after=(Get-CimInstance Win32_OperatingSystem | Select-Object FreePhysicalMemory);processes_cumulative_cpu=(Get-Process | Sort-Object CPU -Descending | Select-Object -First 10 ProcessName,CPU);method='One execution per state/fixture, dev unoptimized, warm OS caches uncontrolled; wall includes process startup, JSON totals do not. PeakWorkingSet64 sampled every 40ms; private memory is sampled, not true peak. CPU time is sampled before exit. No comparative performance verdict or process-RSS cap. No GPU/window work.'} | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $outputRoot 'host-method.json')
$after=@(Source-Hashes);$after | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'source-hashes-after.json')
if (Compare-Object $before $after -Property Path,Hash) { throw 'Source changed during evidence run.' }
'passed' | Set-Content (Join-Path $outputRoot 'result.txt')
Write-Output $outputRoot
