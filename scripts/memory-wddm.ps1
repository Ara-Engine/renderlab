# N8h: sample per-process GPU memory (WDDM "GPU Process Memory" counters: Dedicated Usage,
# Shared Usage) for processes whose name matches -Match, every -IntervalMs, until -StopFile
# exists or -Seconds elapse. One JSON object per line: {t, pid, name, dedicated, shared}.
# The same OS counters cover D3D12 (Unreal) and Vulkan (Renderlab), so totals compare.
param(
    [Parameter(Mandatory = $true)][string]$Match,
    [Parameter(Mandatory = $true)][string]$Output,
    [string]$StopFile = "",
    [int]$IntervalMs = 500,
    [int]$Seconds = 1800
)
$ErrorActionPreference = "SilentlyContinue"
$deadline = (Get-Date).AddSeconds($Seconds)
$writer = [System.IO.StreamWriter]::new($Output, $true, [System.Text.UTF8Encoding]::new($false))
try {
    while ((Get-Date) -lt $deadline) {
        if ($StopFile -and (Test-Path $StopFile)) { break }
        $procs = Get-Process | Where-Object { $_.ProcessName -match $Match }
        if ($procs) {
            $ids = @{}
            foreach ($p in $procs) { $ids[[string]$p.Id] = $p.ProcessName }
            $samples = (Get-Counter -Counter "\GPU Process Memory(*)\Dedicated Usage", "\GPU Process Memory(*)\Shared Usage").CounterSamples
            $rows = @{}
            foreach ($s in $samples) {
                if ($s.InstanceName -match '^pid_(\d+)_') {
                    $id = $Matches[1]
                    if ($ids.ContainsKey($id)) {
                        if (-not $rows.ContainsKey($id)) { $rows[$id] = @{ dedicated = 0.0; shared = 0.0 } }
                        if ($s.Path -match 'dedicated usage$') { $rows[$id].dedicated += $s.CookedValue }
                        else { $rows[$id].shared += $s.CookedValue }
                    }
                }
            }
            $t = (Get-Date).ToUniversalTime().ToString("o")
            foreach ($id in $rows.Keys) {
                $line = '{"t":"' + $t + '","pid":' + $id + ',"name":"' + $ids[$id] + '","dedicated":' + [int64]$rows[$id].dedicated + ',"shared":' + [int64]$rows[$id].shared + '}'
                $writer.WriteLine($line)
            }
            $writer.Flush()
        }
        Start-Sleep -Milliseconds $IntervalMs
    }
}
finally {
    $writer.Close()
}
