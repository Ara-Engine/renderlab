# Run from the repository root. Uses the existing dev target; optional cold builds get a fresh target.
param([switch]$Cold)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot/..").Path
Push-Location $root
$run = Join-Path $root ('measurements/' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $run | Out-Null
$rustPath = Join-Path $root 'src/lib.rs'
$shaderPath = Join-Path $root 'shaders/scene.wgsl'
$rust = [IO.File]::ReadAllBytes($rustPath)
$shader = [IO.File]::ReadAllBytes($shaderPath)
$results = [ordered]@{ commit = (git rev-parse HEAD); build_seconds = [ordered]@{}; launches = @() }
function Build([string]$Name, [string[]]$Extra = @()) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $arguments = @('build','--locked','-p','renderlab-viewer') + $Extra
    $ErrorActionPreference = 'Continue'
    & cargo @arguments *> "$run/$Name.log"
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    $results.build_seconds[$Name] = $clock.Elapsed.TotalSeconds
    if ($exitCode -ne 0) { throw "Build failed: $run/$Name.log" }
}
function Wait-Log([string]$Path, [string]$Pattern) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        if (Test-Path $Path) {
            $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = New-Object IO.StreamReader($stream)
            try { $content = $reader.ReadToEnd() } finally { $reader.Dispose() }
            if ($content -match $Pattern) { return $content }
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Pattern in $Path"
}
$viewer = $null
try {
    if (Test-Path 'D:/AraBuild/render-v3/quiet-window.lock') { throw 'Review the Ara GPU quiet-window lock before measuring.' }
    if ($Cold) { Build 'cold' @('--target-dir', "target/cold-$([Guid]::NewGuid().ToString('N'))") }
    Build 'prepare'
    1..3 | ForEach-Object { Build "noop-$_" }
    $changed = [Text.Encoding]::UTF8.GetString($rust).Replace('yaw: 0.35,','yaw: 0.36,')
    if ($changed -eq [Text.Encoding]::UTF8.GetString($rust)) { throw 'Incremental edit anchor not found' }
    $editClock = [Diagnostics.Stopwatch]::StartNew()
    [IO.File]::WriteAllText($rustPath,$changed)
    Build 'incremental'
    $launchClock = [Diagnostics.Stopwatch]::StartNew()
    $viewer = Start-Process target/debug/renderlab-viewer.exe -WindowStyle Hidden -PassThru -RedirectStandardError "$run/incremental-viewer.log"
    $null = Wait-Log "$run/incremental-viewer.log" 'startup_to_present_ms='
    $results.incremental_edit_to_present_log_seconds = $editClock.Elapsed.TotalSeconds
    $results.incremental_process_to_present_log_seconds = $launchClock.Elapsed.TotalSeconds
    $null = $viewer.CloseMainWindow(); if (!$viewer.WaitForExit(5000)) { throw 'Viewer failed to close' }; $viewer=$null
    [IO.File]::WriteAllBytes($rustPath,$rust)
    Build 'restore'
    foreach ($i in 1..3) {
        $log = "$run/viewer-$i.log"
        $viewer = Start-Process target/debug/renderlab-viewer.exe -WindowStyle Hidden -PassThru -RedirectStandardError $log
        $null = Wait-Log $log 'startup_to_present_ms='
        $modified = [Text.Encoding]::UTF8.GetString($shader).Replace('0.75,0.16,0.055','0.055,0.16,0.75')
        [IO.File]::WriteAllText($shaderPath,$modified)
        $content = Wait-Log $log 'shader_write_to_present_ms='
        $results.launches += $content
        $null = $viewer.CloseMainWindow(); if (!$viewer.WaitForExit(5000)) { throw 'Viewer failed to close' }; $viewer=$null
        [IO.File]::WriteAllBytes($shaderPath,$shader)
    }
} finally {
    if ($null -ne $viewer -and !$viewer.HasExited) { Stop-Process -Id $viewer.Id }
    [IO.File]::WriteAllBytes($rustPath,$rust)
    [IO.File]::WriteAllBytes($shaderPath,$shader)
    $results | ConvertTo-Json -Depth 5 | Set-Content "$run/results.json"
    Pop-Location
}
Get-Content "$run/results.json"
