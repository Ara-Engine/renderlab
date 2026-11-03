param([switch]$Gpu, [switch]$Replay, [string]$Output)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
if (!$Output) { $Output = 'measurements/m4-stream/check-' + (Get-Date -Format 'yyyyMMdd-HHmmss') }
$run = Join-Path $root $Output
if (Test-Path $run) { throw 'Output exists; preserve previous evidence and choose a fresh path.' }
if ($Replay -and !$Gpu) {throw '-Replay requires -Gpu'}
New-Item -ItemType Directory -Force -Path $run | Out-Null
Push-Location $root
$oldImplicit = $env:VK_IMPLICIT_LAYER_PATH
$oldEvidence = $env:RENDERLAB_EVIDENCE_DIR
$env:RENDERLAB_EVIDENCE_DIR = $run
function Run-Cargo([string]$Name, [string[]]$Arguments) {
    if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') { throw 'GPU quiet-window lock present. Review ownership before running.' }
    if ($Gpu -and (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue)) { throw 'Foreground gameplay process present; GPU checks deferred.' }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = 'cargo.exe'
    $process.StartInfo.Arguments = ($Arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }) -join ' '
    $process.StartInfo.WorkingDirectory = $root
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $clock = [Diagnostics.Stopwatch]::StartNew()
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(1800000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "$Name exceeded 30 minutes; only this helper's child was stopped"
        }
        [IO.File]::WriteAllText("$run/$Name.stdout.log", $stdout.Result)
        [IO.File]::WriteAllText("$run/$Name.stderr.log", $stderr.Result)
        @{ arguments = $Arguments; exit = $process.ExitCode; seconds = $clock.Elapsed.TotalSeconds } | ConvertTo-Json | Set-Content "$run/$Name.process.json"
        if ($process.ExitCode -ne 0) { throw "$Name failed. Read $run/$Name.stderr.log and $run/$Name.stdout.log" }
    } finally { $process.Dispose() }
    Write-Output "$Name passed"
}
function Source-Hashes {
    $sources = @('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'viewer/Cargo.toml', 'scripts/m4-runtime-check.ps1', 'scripts/m3-gpu-check.ps1', 'scripts/measure-dev-loop.ps1') + @(Get-ChildItem src,viewer/src,shaders,examples,support,crates/asset,crates/cook -Recurse -File | Select-Object -ExpandProperty FullName)
    Get-FileHash -Path $sources -Algorithm SHA256 | Sort-Object Path | Select-Object Path,Hash
}
try {
    git rev-parse HEAD | Set-Content "$run/commit.txt"
    git status --short | Set-Content "$run/status.txt"
    git diff --stat | Set-Content "$run/diff-stat.txt"
    $before = @(Source-Hashes)
    $before | ConvertTo-Json | Set-Content "$run/source-hashes.json"
    rustc --version | Set-Content "$run/rustc.txt"
    if ($Gpu) {
        if (Get-Process -Name 'VALORANT-Win64-Shipping' -ErrorAction SilentlyContinue) { throw 'Foreground gameplay process present; GPU checks deferred.' }
        $assetRoot = Join-Path $root 'measurements/m3-assets/20261103-005837/cache/assets'
        if ($env:RENDERLAB_M3_ASSETS) { $assetRoot = $env:RENDERLAB_M3_ASSETS }
        $m4Paths=@('measurements/m4-cook/registration-initial/cameras.json','measurements/m4-cook/root-unit-a/cache/virtual-v1/packages/63e6f0d39d2d0b01c1a858c3ba4a5c5704f280268ba482cd28a6fffae349da1d','measurements/m4-cook/root-unit-a/cache/virtual-v1/packages/7c9da3f0eaa6f5f477eeb690908eba639def21948fe9aeafcad7cc4cd0e13eea')
        Get-FileHash -Path $m4Paths -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/m4-input-hashes.json"
        Get-FileHash -Path (Join-Path $assetRoot 'd62f14452111fd34280b1cd56841bf08d79d6d432ee38270c6d44773e25c219c'),(Join-Path $assetRoot '8fabb94f03e50e703e4d4d6eb1371be44c6f9e790c06bae1a65c5ccdae10cd4e') -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/asset-hashes.json"
        if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') { throw 'GPU quiet-window lock present.' }
        $emptyImplicit = Join-Path $root 'measurements/empty-vulkan-implicit'
        New-Item -ItemType Directory -Force -Path $emptyImplicit | Out-Null
        $env:VK_IMPLICIT_LAYER_PATH = $emptyImplicit
        & nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-before.csv"
    }
    @{ gpu_requested = [bool]$Gpu; cargo_jobs = 2; VK_IMPLICIT_LAYER_PATH = $env:VK_IMPLICIT_LAYER_PATH; VK_LAYER_PATH = $env:VK_LAYER_PATH; VK_LOADER_LAYERS_DISABLE = $env:VK_LOADER_LAYERS_DISABLE } | ConvertTo-Json | Set-Content "$run/environment.json"
    Run-Cargo 'fmt' @('fmt', '--all', '--', '--check')
    Run-Cargo 'check' @('check', '--locked', '--workspace', '--all-targets', '-j', '2')
    Run-Cargo 'clippy' @('clippy', '--locked', '--workspace', '--all-targets', '-j', '2', '--', '-D', 'warnings')
    Run-Cargo 'test-compile' @('test', '--locked', '--workspace', '--all-targets', '--no-run', '--message-format=json', '-j', '2')
    $binaries = @(Get-Content "$run/test-compile.stdout.log" | ForEach-Object {
        $item = $_ | ConvertFrom-Json
        if ($item.reason -eq 'compiler-artifact' -and $item.executable) { $item.executable }
    } | Sort-Object -Unique)
    Get-FileHash -Path $binaries -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/test-binary-hashes.json"
    Run-Cargo 'cpu' @('test', '--locked', '--workspace', '--all-targets', '-j', '2', '--', '--test-threads=1')
    if ($Gpu) {
        Run-Cargo 'gpu' @('test', '--locked', '-p', 'renderlab', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture')
        if (!(Select-String -Path "$run/gpu.stdout.log" -Pattern '19 passed; 0 failed; 0 ignored' -Quiet)) {throw 'GPU case count changed: inspect the actual result; no ignored case counts as passed'}
    }
    if ($Replay) {
        $cache = 'measurements/m4-cook/root-unit-a/cache'
        $registration = 'measurements/m4-cook/registration-initial'
        $packages = @('63e6f0d39d2d0b01c1a858c3ba4a5c5704f280268ba482cd28a6fffae349da1d','7c9da3f0eaa6f5f477eeb690908eba639def21948fe9aeafcad7cc4cd0e13eea')
        $identities = @("$registration/registration.json","$registration/cameras.json","$registration/closed-sphere.rla","$registration/material-thin.rla") + @($packages | ForEach-Object {"$cache/virtual-v1/packages/$_"})
        Get-FileHash -Path $identities -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/content-hashes.json"
        foreach ($fixture in @('closed-sphere','material-thin')) {
            $args = @('run','--locked','-p','renderlab','--example','virtual-stream','-j','2','--','--validate','--cache',$cache,'--manifest',"measurements/m4-cook/root-unit-a/$fixture.manifest",'--cameras',"$registration/cameras.json",'--frames','631','--capture','0,150,270,330,630','--output',"$run/$fixture")
            Run-Cargo $fixture $args
        }
        Get-FileHash target/debug/examples/virtual-stream.exe -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/replay-executable.json"
    }
    foreach ($package in @('renderlab','renderlab-viewer','renderlab-asset')) {
        & cargo tree --locked -p $package --edges normal,build | Set-Content "$run/$package-tree.txt"
        if ($LASTEXITCODE -ne 0) {throw 'dependency tree failed'}
        if (Select-String -Path "$run/$package-tree.txt" -Pattern 'meshopt|float-cmp| cc v|gltf| image v' -Quiet) {throw "cooker dependency leaked into $package"}
        if ($package -ne 'renderlab-viewer' -and (Select-String -Path "$run/$package-tree.txt" -Pattern 'serde' -Quiet)) {throw "tooling serialization dependency leaked into $package"}
    }
    $after = @(Source-Hashes)
    $after | ConvertTo-Json | Set-Content "$run/source-hashes-after.json"
    if (Compare-Object $before $after -Property Path,Hash) { throw 'Sources changed during checks; this result has mixed identity.' }
    'passed' | Set-Content "$run/result.txt"
} finally {
    $env:VK_IMPLICIT_LAYER_PATH = $oldImplicit
    $env:RENDERLAB_EVIDENCE_DIR = $oldEvidence
    Pop-Location
}
Write-Output "Check logs: $run"
