<#
.SYNOPSIS
Record game/renderer contention before ANY Unreal launch, including NullRHI imports.
This is a read-only check, not a GPU-slot reservation or permission to steal focus.
#>
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Output, [ValidateRange(0, 64)][double]$MinimumAvailableGiB = 0)
$ErrorActionPreference = 'Stop'
$destination = [IO.Path]::GetFullPath($Output)
$privateRoot = [IO.Path]::GetFullPath('D:/Coding/renderlab-work/ue-m5') + [IO.Path]::DirectorySeparatorChar
if (-not $destination.StartsWith($privateRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Preflight evidence must stay under the private UE project.'
}
if (Test-Path -LiteralPath $destination) { throw 'Refusing to overwrite a preflight record.' }
$processes = @(Get-CimInstance Win32_Process)
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class UeForegroundReadOnly {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
}
'@
$ueForegroundWindow = [UeForegroundReadOnly]::GetForegroundWindow()
$ueForegroundProcess = [uint32]0
[UeForegroundReadOnly]::GetWindowThreadProcessId($ueForegroundWindow, [ref]$ueForegroundProcess) | Out-Null
$gamePattern = '(?i)valorant|eldenring|cyberpunk|helldivers|cs2\.exe|fortniteclient|overwatch|arcraiders|bf6\.exe|league of legends|r5apex|dota2|rocketleague|minecraft|win64-shipping|shipping\.exe'
$rendererPattern = '(?i)^(renderlab|m6-shadows|quarry-resets|fresh-tests|screen-|sel2shadow|UnrealEditor|UnrealGame|ShaderCompileWorker|UnrealLightmass)'
$conflicts = @($processes | Where-Object {
    $name = $_.Name
    $path = [string]$_.ExecutablePath
    $name -match $gamePattern -or $name -match $rendererPattern -or
    ($path -match '(?i)\\(renderlab-work|renderlab)\\.*\.exe$' -and
     $name -notmatch '(?i)^(cargo|rustc|rust-analyzer|python|py|powershell|pwsh)\.exe$') -or
    ($path -match '(?i)\\steamapps\\common\\' -and $name -notmatch '(?i)wallpaper|crash|helper')
} | Select-Object Name, ProcessId, ExecutablePath)
$builds = @($processes | Where-Object { $_.Name -match '(?i)^(cargo|rustc|cl|link|blender)\.exe$' } |
    Select-Object Name, ProcessId, ExecutablePath)
$gpu = @(& nvidia-smi --query-gpu=name,driver_version,utilization.gpu,memory.used,memory.total,clocks.gr,temperature.gpu,power.draw --format=csv)
if ($LASTEXITCODE -ne 0) { throw 'GPU inventory failed; no launch permitted.' }
$memory = Get-CimInstance Win32_OperatingSystem | Select-Object FreePhysicalMemory, TotalVisibleMemorySize
$availableMiB = (Get-Counter -Counter '\Memory\Available MBytes').CounterSamples.CookedValue
$enoughMemory = if ($MinimumAvailableGiB -gt 0) { $availableMiB -ge $MinimumAvailableGiB * 1024 } else { [int64]$memory.FreePhysicalMemory -ge 8 * 1024 * 1024 }
$record = [ordered]@{
    captured_utc = [DateTime]::UtcNow.ToString('o')
    conflicts = $conflicts
    cpu_builds = $builds
    gpu = $gpu
    memory = $memory
    foreground_read_only = @{ window_handle = $ueForegroundWindow.ToInt64(); process_id = $ueForegroundProcess }
    minimum_free_ram_gib = if ($MinimumAvailableGiB -gt 0) { $null } else { 8 }
    minimum_available_ram_gib = if ($MinimumAvailableGiB -gt 0) { $MinimumAvailableGiB } else { $null }
    available_memory_mib = $availableMiB
    available_memory_counter = '\Memory\Available MBytes'
    status = if ($conflicts.Count -or -not $enoughMemory) { 'BLOCKED' } else { 'NO_KNOWN_PROCESS_CONFLICTS' }
    scope = 'One process snapshot only. Recheck immediately before launch; coordinate the shared GPU slot. No focus/input/capture performed.'
}
$parent = Split-Path -Parent $destination
New-Item -ItemType Directory -Force -Path $parent | Out-Null
$record | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $destination -Encoding UTF8
if ($conflicts.Count) {
    $conflicts | Format-Table Name, ProcessId -AutoSize
    throw 'Deferred: game or renderer process detected. Do not launch Unreal or use the GPU.'
}
if (-not $enoughMemory) { throw 'Deferred: RAM is below the recorded launch floor. Do not launch Unreal.' }
Write-Output "Preflight recorded at $destination. Coordinate slot ownership before launching."
