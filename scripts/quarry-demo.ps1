<#
.SYNOPSIS
Open the cooked quarry with an existing viewer binary. Build only with -Build.
.EXAMPLE
.\scripts\quarry-demo.ps1 -Profile release -QualityMode streamed -Samples 4
.EXAMPLE
.\scripts\quarry-demo.ps1 -Profile dev -Build -Validate
#>
[CmdletBinding()]
param(
    [Alias('Profile')][ValidateSet('dev', 'release')][string]$BuildProfile = 'release',
    [ValidateSet('streamed', 'full')][string]$QualityMode = 'streamed',
    [ValidateSet(1, 4)][int]$Samples = 4,
    [string]$Asset = 'measurements/quarry-scan/cache/assets/86bdd8afdf69bace8a08d3974e8e9da9af20e6b261880b9728730d9f3387b0b3',
    [string]$Manifest = 'measurements/quarry-scan/quarry-virtual.manifest',
    [string]$Cache = 'measurements/quarry-scan/cache',
    [string]$Limits = 'measurements/quarry-scan/limits.json',
    [string]$Cameras = 'measurements/quarry-scan/cameras.json',
    [ValidateSet('interactive', 'held', 'path')][string]$CameraMode = 'interactive',
    [ValidateRange(0, 9999)][int]$Pose = 0,
    [ValidateSet('baseline', 'responsive')][string]$RuntimePolicy = 'responsive',
    [ValidateRange(1, 2147483647)][int]$ReadSlots = 128,
    [ValidateRange(1, 2147483647)][int]$UploadsPerTick = 64,
    [string]$VariantDirectory = 'measurements/quarry-tuning/demo',
    [string]$Binary,
    [switch]$Build,
    [switch]$Validate,
    [switch]$Progressive,
    [switch]$PrintCommand
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$previousImplicitLayers = $env:VK_IMPLICIT_LAYER_PATH
Push-Location -LiteralPath $repo
try {
    if ($Build -and ($Binary -or $PrintCommand)) {
        throw '-Build requires the default binary and cannot be combined with -PrintCommand.'
    }
    if ($RuntimePolicy -eq 'baseline' -and
        ($PSBoundParameters.ContainsKey('ReadSlots') -or $PSBoundParameters.ContainsKey('UploadsPerTick'))) {
        throw 'Use -RuntimePolicy responsive to apply read/upload overrides.'
    }
    $inputs = @($Limits, $Cameras)
    if ($QualityMode -eq 'full') { $inputs += $Asset }
    else { $inputs += $Manifest; $inputs += $Cache }
    foreach ($inputPath in $inputs) {
        if (-not (Test-Path -LiteralPath $inputPath)) {
            throw "Missing cooked input '$inputPath'. Pass its existing path. This launcher does not import assets."
        }
    }
    $variantJson = & py "$PSScriptRoot/scan-tuning-policy.py" --limits $Limits --cameras $Cameras `
        --output $VariantDirectory --read-slots $ReadSlots --uploads-per-tick $UploadsPerTick --pose $Pose
    if ($LASTEXITCODE -ne 0) { throw 'Could not prepare the local policy/camera variants.' }
    $variant = $variantJson | ConvertFrom-Json
    $effectiveLimits = if ($RuntimePolicy -eq 'responsive') { $variant.limits } else { $Limits }
    $effectiveCameras = if ($CameraMode -ne 'path') { $variant.camera } else { $Cameras }
    if (-not $Binary) {
        $targetDirectory = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
        $profileDirectory = if ($BuildProfile -eq 'dev') { 'debug' } else { 'release' }
        $Binary = Join-Path $targetDirectory "$profileDirectory/renderlab-viewer.exe"
    }
    $viewerArgs = @('--limits', $effectiveLimits, '--cameras', $effectiveCameras,
        '--shader', (Join-Path $repo 'shaders/scene.wgsl'), '--samples', "$Samples")
    if ($CameraMode -eq 'interactive') { $viewerArgs += '--interactive-camera' }
    if ($QualityMode -eq 'full') { $viewerArgs += @('--asset', $Asset) }
    else { $viewerArgs += @('--virtual-manifest', $Manifest, '--cache', $Cache) }
    if ($Validate) { $viewerArgs += '--validate' }
    if ($Progressive) {
        if ($QualityMode -eq 'full') { throw '-Progressive requires -QualityMode streamed.' }
        $viewerArgs += '--progressive'
    }
    Write-Host "quarry profile=$BuildProfile geometry=$QualityMode samples=$Samples policy=$RuntimePolicy validate=$Validate build=$Build"
    Write-Host "canonical_limits_sha256=$($variant.baseline_sha256)"
    $commandParts = @($Binary) + $viewerArgs | ForEach-Object { "'" + $_.Replace("'", "''") + "'" }
    Write-Host ('& ' + ($commandParts -join ' '))
    if ($PrintCommand) { return }
    if ($Build) {
        & cargo build --locked -j 2 -p renderlab-viewer --profile $BuildProfile
        if ($LASTEXITCODE -ne 0) { throw 'Viewer build failed.' }
    }
    if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
        throw "Viewer binary missing '$Binary'. Build explicitly with -Build when the build/GPU slot is free."
    }
    $implicitLayers = Join-Path (Split-Path $variant.limits -Parent) 'empty-vulkan-implicit'
    New-Item -ItemType Directory -Force -Path $implicitLayers | Out-Null
    $env:VK_IMPLICIT_LAYER_PATH = $implicitLayers
    & (Resolve-Path -LiteralPath $Binary).Path @viewerArgs
    if ($LASTEXITCODE -ne 0) { throw "Viewer exited with code $LASTEXITCODE." }
}
finally {
    $env:VK_IMPLICIT_LAYER_PATH = $previousImplicitLayers
    Pop-Location
}
