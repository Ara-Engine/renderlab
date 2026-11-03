[CmdletBinding()]
param(
    [ValidateSet('Source', 'Fixture', 'Images', 'Quarry', 'Retained')][string]$Stage = 'Source',
    [ValidateSet(1, 4)][int]$Samples = 1,
    [ValidateSet('static', 'camera', 'sun', 'spot', 'point', 'blocker')][string]$Motion = 'static',
    [int]$Frames = 3,
    [int]$Resolution = 1024,
    [string]$Output,
    [switch]$Build,
    [switch]$Cost
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$run = Join-Path $repo ('.refs/m6-gpu/check-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Force -Path $run | Out-Null
$commands = [System.Collections.Generic.List[object]]::new()
function Invoke-Checked([string]$Exe, [string[]]$Arguments) {
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $log = Join-Path $run ('command-' + $commands.Count + '.log')
    $priorAction = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments 2>&1 | Tee-Object -FilePath $log
        $exitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $priorAction }
    $commands.Add([pscustomobject]@{ command = $Exe; arguments = $Arguments; exit = $exitCode; elapsed_seconds = $timer.Elapsed.TotalSeconds; log = $log })
    if ($exitCode -ne 0) { throw "Command failed with exit $exitCode. See $log" }
}
$owned = @('src/shadows.rs', 'src/shadow_gpu.rs', 'src/shadow_plan.rs', 'src/shadow_plan_tests.rs',
    'src/shadow_tests.rs', 'src/tests.rs', 'src/renderer.rs', 'src/lighting_gpu.rs', 'src/virtual_gpu.rs',
    'src/observe.rs', 'src/lib.rs', 'support/shadow_fixture.rs', 'support/lighting_settings.rs', 'support/lighting_controls.rs',
    'examples/m6-shadows.rs', 'viewer/src/main.rs')
$previousLayers = $env:VK_IMPLICIT_LAYER_PATH
Push-Location -LiteralPath $repo
try {
    if ($Stage -eq 'Source') {
        Invoke-Checked rustfmt (@('--edition', '2024', '--config', 'skip_children=true', '--check') + $owned)
        Invoke-Checked git @('diff', '--check', '--', 'shaders/scene.wgsl')
        $deps = Join-Path $repo 'target/debug/deps'
        $glam = Get-ChildItem -LiteralPath $deps -Filter 'libglam-*.rlib' | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        $naga = Get-ChildItem -LiteralPath $deps -Filter 'libnaga-*.rlib' | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if (-not $glam -or -not $naga) { throw 'Source checks require the existing main debug dependency cache. This script will not build a cache.' }
        $mesh = Get-Content -Raw -LiteralPath src/mesh.rs
        $cameraStart = $mesh.IndexOf('/// Right-handed camera')
        $camera = $mesh.Substring($cameraStart, $mesh.IndexOf('pub(crate) fn matrix_bytes') - $cameraStart)
        $path = $repo.Replace('\', '/')
        $geometry = Get-Content -Raw -LiteralPath src/shadow_gpu.rs
        $geometryStart = $geometry.IndexOf('pub(super) fn world_bounds')
        $geometryEnd = $geometry.IndexOf('impl Renderer {', $geometryStart)
        $geometryFunctions = $geometry.Substring($geometryStart, $geometryEnd - $geometryStart)
        $geometryTests = $geometry.Substring($geometry.LastIndexOf('#[cfg(test)]'))
        $cpuSource = @"
extern crate glam;
pub use glam::{Mat4,Vec3};
#[derive(Debug)] pub enum Error { InvalidInput(&'static str) }
$camera
#[path="$path/src/lighting.rs"] mod lighting;
pub use lighting::Light;
#[path="$path/src/shadows.rs"] mod shadows;
#[path="$path/src/shadow_plan.rs"] mod shadow_plan;
mod adapter_geometry {
use super::*;
use crate::shadow_plan::Bounds;
#[derive(Clone, Copy)] pub(super) struct Aabb { min:[f32;3],max:[f32;3] }
$geometryFunctions
$geometryTests
}
"@
        $cpuPath = Join-Path $run 'cpu.rs'
        $cpuExe = Join-Path $run 'cpu.exe'
        Set-Content -LiteralPath $cpuPath -Value $cpuSource
        Invoke-Checked rustc @('--edition=2024', '--test', $cpuPath, '--extern', "glam=$($glam.FullName)", '-L', "dependency=$deps", '-A', 'dead_code', '-o', $cpuExe)
        Invoke-Checked $cpuExe @()
        $shaderSource = @'
extern crate naga;
fn main() {
    let source=std::fs::read_to_string("shaders/scene.wgsl").unwrap();
    let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
    println!("CPU WGSL parse/validation passed; no device layout or raster verification");
}
'@
        $shaderPath = Join-Path $run 'shader.rs'
        $shaderExe = Join-Path $run 'shader.exe'
        Set-Content -LiteralPath $shaderPath -Value $shaderSource
        Invoke-Checked rustc @('--edition=2024', $shaderPath, '--extern', "naga=$($naga.FullName)", '-L', "dependency=$deps", '-o', $shaderExe)
        Invoke-Checked $shaderExe @()
    } else {
        # These stages require the root's GPU/heavy-build slot grant.
        Get-PSDrive -Name C,D | Select-Object Name,Used,Free
        Get-CimInstance Win32_OperatingSystem | Select-Object TotalVisibleMemorySize,FreePhysicalMemory
        $env:VK_IMPLICIT_LAYER_PATH = Join-Path $repo 'measurements/empty-vulkan-implicit'
        if ($Stage -eq 'Fixture') {
            Invoke-Checked cargo @('test', '--locked', '-p', 'renderlab', '--lib', 'shadow_', '-j', '2', '--', '--test-threads=1')
            Invoke-Checked cargo @('test', '--locked', '-p', 'renderlab', '--lib', 'gpu_shadow_', '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture')
        } elseif ($Stage -eq 'Retained') {
            Invoke-Checked cargo @('clippy', '--locked', '-p', 'renderlab', '-p', 'renderlab-viewer', '--all-targets', '-j', '2', '--', '-D', 'warnings')
            Invoke-Checked cargo @('test', '--locked', '-p', 'renderlab', '--lib', '-j', '2', '--', '--test-threads=1')
            foreach ($filter in @('gpu_lit_', 'gpu_images_uploads_reload_and_timing', 'gpu_material_values_sampler_normals_and_sidedness', 'gpu_assets_aa_reload_and_reuse', 'gpu_aa_edge_reference', 'gpu_material_target_replacement_overlap', 'gpu_replacement_stress', 'gpu_batched_culling_oracle')) {
                Invoke-Checked cargo @('test', '--locked', '-p', 'renderlab', '--lib', $filter, '-j', '2', '--', '--ignored', '--test-threads=1', '--nocapture')
            }
        } else {
            if ($Build) { Invoke-Checked cargo @('build', '--locked', '-p', 'renderlab', '--example', 'm6-shadows', '--release', '-j', '2') }
            if (-not $Output) { $Output = "measurements/m6-gpu/$($Stage.ToLower())-$Motion-$($Samples)x" }
            $arguments = @('--validate', '--samples', "$Samples", '--frames', "$Frames", '--motion', $Motion, '--shadow-resolution', "$Resolution", '--output', $Output)
            if ($Cost) { $arguments += '--cost' }
            if ($Stage -eq 'Quarry') {
                $arguments += @('--asset', 'measurements/quarry-quality/detail/full.rla', '--limits', 'measurements/quarry-quality/detail/limits.json', '--cameras', 'measurements/quarry-scan/cameras.json', '--lighting', 'fixtures/quarry-lighting.json')
            }
            Invoke-Checked (Join-Path $repo 'target/release/examples/m6-shadows.exe') $arguments
        }
    }
} finally {
    $env:VK_IMPLICIT_LAYER_PATH = $previousLayers
    $identities = @{}
    foreach ($file in ($owned + @('shaders/scene.wgsl', 'Cargo.lock'))) { $identities[$file] = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $repo $file)).Hash.ToLowerInvariant() }
    [pscustomobject]@{ stage=$Stage; time=(Get-Date -Format o); commands=$commands; sources=$identities } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $run 'result.json')
    Pop-Location
    Write-Host "Evidence $run"
}
