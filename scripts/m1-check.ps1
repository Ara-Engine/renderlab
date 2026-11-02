param([switch]$Gpu)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
Push-Location $root
$run = Join-Path $root ('measurements/m1-implementation/check-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $run | Out-Null
function Run-Cargo([string]$Name, [string[]]$Arguments) {
    if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') { throw 'Quiet-window lock present. Review ownership before running.' }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = 'cargo.exe'
    $process.StartInfo.Arguments = $Arguments -join ' '
    $process.StartInfo.WorkingDirectory = $root
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $process.WaitForExit()
        [IO.File]::WriteAllText("$run/$Name.stdout.log", $stdout.Result)
        [IO.File]::WriteAllText("$run/$Name.stderr.log", $stderr.Result)
        if ($process.ExitCode -ne 0) { throw "$Name failed. Read $run/$Name.stderr.log and $run/$Name.stdout.log" }
    } finally { $process.Dispose() }
    Write-Output "$Name passed"
}
try {
    git rev-parse HEAD | Set-Content "$run/commit.txt"
    git diff --stat | Set-Content "$run/diff-stat.txt"
    $sources = @('Cargo.toml', 'viewer/Cargo.toml', 'Cargo.lock', 'shaders/scene.wgsl') + @(Get-ChildItem src,viewer/src,support -Recurse -File -Filter '*.rs' | Select-Object -ExpandProperty FullName)
    Get-FileHash -Path $sources -Algorithm SHA256 | Select-Object Path,Hash | ConvertTo-Json | Set-Content "$run/source-hashes.json"
    rustc --version | Set-Content "$run/rustc.txt"
    @{ VK_IMPLICIT_LAYER_PATH = $env:VK_IMPLICIT_LAYER_PATH; VK_LAYER_PATH = $env:VK_LAYER_PATH; VK_LOADER_LAYERS_DISABLE = $env:VK_LOADER_LAYERS_DISABLE } | ConvertTo-Json | Set-Content "$run/vulkan-environment.json"
    & nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-before.csv"
    Run-Cargo 'fmt' @('fmt', '--all', '--', '--check')
    Run-Cargo 'check' @('check', '--locked', '--workspace', '--all-targets', '-j', '2')
    Run-Cargo 'clippy' @('clippy', '--locked', '--workspace', '--all-targets', '-j', '2', '--', '-D', 'warnings')
    Run-Cargo 'cpu' @('test', '--locked', '-p', 'renderlab', '-j', '2', '--', '--test-threads=1')
    if ($Gpu) { Run-Cargo 'gpu' @('test', '--locked', '-p', 'renderlab', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture') }
} finally { Pop-Location }
Write-Output "Check logs: $run"
