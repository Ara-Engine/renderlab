param([string]$Output = "measurements/m3-assets/$(Get-Date -Format yyyyMMdd-HHmmss)")
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Set-Location $repoRoot
if (Test-Path -LiteralPath $Output) { throw 'Choose a fresh output directory so the first cook is a cache miss.' }
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$outputRoot = (Resolve-Path -LiteralPath $Output).Path
function Source-Hashes {
    $paths = @('Cargo.toml', 'Cargo.lock', 'scripts/m3-asset-check.ps1') + @(Get-ChildItem crates/asset,crates/cook -Recurse -File | Select-Object -ExpandProperty FullName)
    Get-FileHash -Path $paths -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash
}
$before = @(Source-Hashes)
$before | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'source-hashes.json')

function Invoke-CargoCheck([string]$Label, [string[]]$Arguments) {
    $ErrorActionPreference = 'Continue'
    $timer = [Diagnostics.Stopwatch]::StartNew()
    & cargo @Arguments 2>&1 | ForEach-Object { $_.ToString() } | Set-Content (Join-Path $outputRoot "$Label.log")
    if ($LASTEXITCODE -ne 0) { throw "$Label failed. Read $outputRoot/$Label.log" }
    $timer.Stop()
    [PSCustomObject]@{ command = "cargo $($Arguments -join ' ')"; elapsed_ms = $timer.Elapsed.TotalMilliseconds }
}

$checks = @()
$checks += Invoke-CargoCheck 'fmt' @('fmt', '--package', 'renderlab-asset', '--package', 'renderlab-cook', '--', '--check')
$checks += Invoke-CargoCheck 'check' @('check', '--locked', '-j', '2', '-p', 'renderlab-asset', '-p', 'renderlab-cook', '--all-targets')
$checks += Invoke-CargoCheck 'tests' @('test', '--locked', '-j', '2', '-p', 'renderlab-asset', '-p', 'renderlab-cook')
$checks += Invoke-CargoCheck 'clippy' @('clippy', '--locked', '-j', '2', '-p', 'renderlab-asset', '-p', 'renderlab-cook', '--all-targets', '--', '-D', 'warnings')
$checks | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $outputRoot 'checks.json')

foreach ($package in @('renderlab', 'renderlab-viewer', 'renderlab-asset', 'renderlab-cook')) {
    $tree = & cargo tree -p $package --edges normal
    if ($LASTEXITCODE -ne 0) { throw "cargo tree failed for $package" }
    $tree | Set-Content (Join-Path $outputRoot "$package-tree.txt")
    if ($package -ne 'renderlab-cook' -and ($tree -match '\b(gltf|image|png|zune-jpeg|sha2) v')) { throw "Importer/codec/hash dependency leaked into $package" }
}

$sources = Join-Path $outputRoot 'sources'
$ErrorActionPreference = 'Continue'
$fixtureMessages = & cargo run -j 2 -p renderlab-cook -- fixtures $sources 2>&1 | ForEach-Object { $_.ToString() }
$fixtureExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
$fixtureMessages | Where-Object { $_ -match '^\{' } | Set-Content (Join-Path $outputRoot 'sources.jsonl')
$fixtureMessages | Where-Object { $_ -notmatch '^\{' } | Set-Content (Join-Path $outputRoot 'cli-compile.log')
if ($fixtureExit -ne 0) { throw 'Fixture CLI failed.' }
$executable = (Resolve-Path target/debug/renderlab-cook.exe).Path
$cache = Join-Path $outputRoot 'cache'
$records = @()
foreach ($name in @('instanced-boxes', 'curved-thin')) {
    $sourcePath = Join-Path $sources "$name.glb"
    $manifest = Join-Path $outputRoot "$name.manifest"
    foreach ($state in @('cold', 'warm')) {
        $record = & $executable cook $sourcePath $cache $manifest
        if ($LASTEXITCODE -ne 0) { throw "$name $state CLI failed." }
        $parsed = $record | ConvertFrom-Json
        if ($parsed.cache_hit -ne ($state -eq 'warm')) { throw 'Unexpected cache state.' }
        $records += [PSCustomObject]@{ asset = $name; cache = $state; result = $parsed }
    }
    & $executable inspect $cache $manifest > (Join-Path $outputRoot "$name-inspect.json")
    if ($LASTEXITCODE -ne 0) { throw 'Manifest load CLI failed.' }
    & py -3 crates/cook/tools/inspect_assets.py $sourcePath $cache $manifest > (Join-Path $outputRoot "$name-independent.json")
    if ($LASTEXITCODE -ne 0) { throw 'Independent inspection failed.' }
}
$records | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $outputRoot 'cli-runs.json')
$cpu = Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors
$os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, TotalVisibleMemorySize
$processes = Get-Process | Sort-Object CPU -Descending | Select-Object -First 8 ProcessName, CPU
[PSCustomObject]@{
    timestamp = (Get-Date).ToString('o'); head = (& git rev-parse HEAD); rustc = (& rustc -Vv) -join "`n";
    binary = $executable; binary_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash;
    cpu = $cpu; os = $os; processes_cumulative_cpu = $processes;
    method = 'One run per asset/cache state. Debug unoptimized. First cache is empty, OS caches uncontrolled. Source generation is outside timed cook. CLI internal totals exclude process startup. No rendering or GPU work. Checks use existing target caches. No comparative performance verdict.'
} | ConvertTo-Json -Depth 7 | Set-Content (Join-Path $outputRoot 'host-method.json')
$after = @(Source-Hashes)
$after | ConvertTo-Json | Set-Content (Join-Path $outputRoot 'source-hashes-after.json')
if (Compare-Object $before $after -Property Path,Hash) { throw 'Sources changed during asset checks.' }
'passed' | Set-Content (Join-Path $outputRoot 'result.txt')
Write-Output $outputRoot
