param([switch]$Gpu)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
$run = Join-Path $root ('measurements/m2-implementation/check-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $run | Out-Null
Push-Location $root
$oldImplicit = $env:VK_IMPLICIT_LAYER_PATH
function Run-Cargo([string]$Name, [string[]]$Arguments) {
    if ($Gpu -and (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock')) { throw 'GPU quiet-window lock present. Review ownership before running.' }
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
    $sources = @('Cargo.toml', 'Cargo.lock', 'viewer/Cargo.toml', 'scripts/m2-check.ps1') + @(Get-ChildItem src,viewer/src,shaders,examples,support -Recurse -File | Select-Object -ExpandProperty FullName)
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
    if ($Gpu) { Run-Cargo 'gpu' @('test', '--locked', '-p', 'renderlab', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture') }
    $after = @(Source-Hashes)
    $after | ConvertTo-Json | Set-Content "$run/source-hashes-after.json"
    if (Compare-Object $before $after -Property Path,Hash) { throw 'Sources changed during checks; this result has mixed identity.' }
    'passed' | Set-Content "$run/result.txt"
} finally {
    $env:VK_IMPLICIT_LAYER_PATH = $oldImplicit
    Pop-Location
}
Write-Output "Check logs: $run"
