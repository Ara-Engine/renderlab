param([switch]$Cold, [int]$Runs = 5)
$ErrorActionPreference = 'Stop'
if ($Runs -lt 1) { throw 'Runs must be positive' }
$root = (Resolve-Path "$PSScriptRoot/..").Path
Push-Location $root
$run = Join-Path $root ('measurements/dev-loop/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run -Force | Out-Null
$utf8 = [Text.UTF8Encoding]::new($false)
$rustPath = Join-Path $root 'src/renderer.rs'
$shaderPath = Join-Path $root 'shaders/scene.wgsl'
$rustStream = $null
$shaderStream = $null
$rust = $null
$shader = $null
$viewer = $null
$results = [ordered]@{
    definition = 'M1 asynchronous first completion-observed and present-call proxies, not scanout; not comparable to M0 wait-before-present'
    commit = (git rev-parse HEAD)
    diff = (git diff --stat | Out-String)
    profile = 'dev'
    cargo_jobs = 2
    scene = 'two indexed cubes and a triangle; Depth32Float; one sample; 960x640 initial window'
    builds = [ordered]@{}
    launches = @()
    binaries = [ordered]@{}
    vulkan_implicit_layer_path = $env:VK_IMPLICIT_LAYER_PATH
    success = $false
    source_hashes = @(Get-FileHash -Path (@('Cargo.toml', 'Cargo.lock', 'viewer/Cargo.toml') + @(Get-ChildItem src,viewer/src,shaders -Recurse -File | Select-Object -ExpandProperty FullName)) -Algorithm SHA256 | Select-Object Path,Hash)
}
function Assert-Quiet {
    if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') { throw 'Quiet-window lock present. Review its owner before running.' }
}
function Read-Bytes([IO.FileStream]$Stream) {
    $bytes = [byte[]]::new($Stream.Length)
    $Stream.Position = 0
    $count = $Stream.Read($bytes, 0, $bytes.Length)
    if ($count -ne $bytes.Length) { throw 'Incomplete source backup' }
    return ,$bytes
}
function Write-Bytes([IO.FileStream]$Stream, [byte[]]$Bytes) {
    $Stream.Position = 0
    $Stream.SetLength($Bytes.Length)
    $Stream.Write($Bytes, 0, $Bytes.Length)
    $Stream.Flush($true)
}
function Edit-Once([byte[]]$Bytes, [string]$Anchor, [string]$Replacement) {
    $text = $utf8.GetString($Bytes)
    if ([regex]::Matches($text, [regex]::Escape($Anchor)).Count -ne 1) { throw "Anchor must occur exactly once: $Anchor" }
    return ,$utf8.GetBytes($text.Replace($Anchor, $Replacement))
}
function Build([string]$Name, [string[]]$Extra = @()) {
    Assert-Quiet
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $arguments = @('build', '--locked', '-j', '2', '-p', 'renderlab-viewer') + $Extra
    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = 'cargo.exe'
    $process.StartInfo.Arguments = ($arguments | ForEach-Object { '"' + $_.Replace('"', '\"') + '"' }) -join ' '
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
        $clock.Stop()
        $results.builds[$Name] = @{ seconds = $clock.Elapsed.TotalSeconds; exit = $process.ExitCode; arguments = $arguments; timing = 'process start through exit, includes launch overhead' }
        [IO.File]::WriteAllText("$run/$Name.stdout.log", $stdout.Result)
        [IO.File]::WriteAllText("$run/$Name.stderr.log", $stderr.Result)
        if ($process.ExitCode -ne 0) { throw "Build failed. Read $run/$Name.stderr.log" }
    } finally { $process.Dispose() }
}
function Wait-Log([string]$Path, [string]$Pattern) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        if (Test-Path $Path) {
            $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = [IO.StreamReader]::new($stream)
            try { $content = $reader.ReadToEnd() } finally { $reader.Dispose() }
            if ($content -match 'viewer_stopped|panicked') { throw "Viewer failed. Read $Path" }
            if ($content -match $Pattern) { return $content }
        }
        if ($script:viewer.HasExited) { throw "Viewer exited before required work. Read $Path" }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Pattern in $Path"
}
function Launch([string]$Name) {
    Assert-Quiet
    $results.binaries[$Name] = (Get-FileHash target/debug/renderlab-viewer.exe -Algorithm SHA256).Hash
    $script:viewer = Start-Process target/debug/renderlab-viewer.exe -ArgumentList @('--seconds', '25') -WindowStyle Hidden -PassThru -RedirectStandardOutput "$run/$Name.stdout.log" -RedirectStandardError "$run/$Name.log"
    $null = $script:viewer.Handle
}
function Close-Viewer {
    if (!$script:viewer.HasExited) {
        $null = $script:viewer.CloseMainWindow()
        if (!$script:viewer.WaitForExit(5000)) { throw 'Viewer failed to close within five seconds' }
    }
    if ($script:viewer.ExitCode -ne 0) { throw "Viewer failed with exit $($script:viewer.ExitCode)" }
    $script:viewer = $null
}
try {
    Assert-Quiet
    $rustStream = [IO.File]::Open($rustPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
    $rust = Read-Bytes $rustStream
    $shaderStream = [IO.File]::Open($shaderPath, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::Read)
    $shader = Read-Bytes $shaderStream
    $changedRust = Edit-Once $rust 'const CLEAR_COLOR: [f64; 3] = [0.018, 0.025, 0.04];' 'const CLEAR_COLOR: [f64; 3] = [0.019, 0.025, 0.04];'
    $changedShader = Edit-Once $shader 'const TINT: vec3<f32> = vec3<f32>(1.0, 1.0, 1.0);' 'const TINT: vec3<f32> = vec3<f32>(0.5, 1.0, 1.0);'
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $results.rust_sha256_before = [BitConverter]::ToString($hasher.ComputeHash($rust)).Replace('-', '')
        $results.shader_sha256_before = [BitConverter]::ToString($hasher.ComputeHash($shader)).Replace('-', '')
    } finally { $hasher.Dispose() }
    & nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used --format=csv | Set-Content "$run/gpu-before.csv"
    Get-Process | Sort-Object CPU -Descending | Select-Object -First 20 Id, ProcessName, CPU, WorkingSet64 | ConvertTo-Json | Set-Content "$run/processes-before.json"
    if ($Cold) { Build 'clean-target' @('--target-dir', "target/dev-loop-clean-$([Guid]::NewGuid().ToString('N'))") }
    Build 'prepare'
    1..$Runs | ForEach-Object { Build "noop-$_" }
    $editClock = [Diagnostics.Stopwatch]::StartNew()
    Write-Bytes $rustStream $changedRust
    Build 'incremental'
    $launchClock = [Diagnostics.Stopwatch]::StartNew()
    Launch 'incremental-viewer'
    $null = Wait-Log "$run/incremental-viewer.log" 'startup_to_completion_observed_proxy_ms='
    $null = Wait-Log "$run/incremental-viewer.log" 'startup_to_present_call_proxy_ms='
    $results.incremental_edit_to_observed_log_seconds = $editClock.Elapsed.TotalSeconds
    $results.incremental_process_to_observed_log_seconds = $launchClock.Elapsed.TotalSeconds
    Close-Viewer
    Write-Bytes $rustStream $rust
    Build 'restore'
    foreach ($i in 1..$Runs) {
        Launch "startup-shader-$i"
        $log = "$run/startup-shader-$i.log"
        $null = Wait-Log $log 'startup_to_completion_observed_proxy_ms='
        $null = Wait-Log $log 'startup_to_present_call_proxy_ms='
        Write-Bytes $shaderStream $changedShader
        $null = Wait-Log $log 'shader_write_to_present_call_proxy_ms='
        $content = Wait-Log $log 'shader_write_to_completion_observed_proxy_ms='
        if ($content -notmatch 'reload_compile_ms=') { throw 'No successful reload was reported' }
        $results.launches += @{ log = $log; raw = $content }
        Close-Viewer
        Write-Bytes $shaderStream $shader
    }
    $results.success = $true
} finally {
    if ($null -ne $viewer -and !$viewer.HasExited) { Stop-Process -Id $viewer.Id }
    if ($null -ne $rustStream) {
        if ($null -ne $rust) { Write-Bytes $rustStream $rust }
        $rustStream.Dispose()
    }
    if ($null -ne $shaderStream) {
        if ($null -ne $shader) { Write-Bytes $shaderStream $shader }
        $shaderStream.Dispose()
    }
    $results.rust_sha256_after = (Get-FileHash $rustPath -Algorithm SHA256).Hash
    $results.shader_sha256_after = (Get-FileHash $shaderPath -Algorithm SHA256).Hash
    $results | ConvertTo-Json -Depth 7 | Set-Content "$run/results.json"
    Pop-Location
}
if ($results.rust_sha256_before -ne $results.rust_sha256_after -or $results.shader_sha256_before -ne $results.shader_sha256_after) { throw 'Exact source restoration failed' }
Write-Output "Development-loop logs: $run/results.json"
