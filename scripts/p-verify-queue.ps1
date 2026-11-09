param(
    [int]$WaitSeconds = 3600,
    [Parameter(Mandatory = $true)][string[]]$TaskCommand
)

# Join the existing stillkit build queue without stopping or changing its work.
# Once acquired, admission may wait only 30 seconds; a GPU lease is <=15 minutes.
$taskMutex = [System.Threading.Mutex]::new($false, 'Global\stillkit-cargo-one-at-a-time')
$taskAcquired = $false
$taskExit = 3
$taskAudit = 'D:/Coding/renderlab-work/p-verify/mutex.jsonl'
$taskClock = [System.Diagnostics.Stopwatch]::new()
try {
    try {
        $taskAcquired = $taskMutex.WaitOne([TimeSpan]::FromSeconds($WaitSeconds))
    } catch [System.Threading.AbandonedMutexException] {
        $taskAcquired = $true
    }
    if ($taskAcquired) {
        $taskClock.Start()
        Add-Content -LiteralPath $taskAudit -Encoding utf8 -Value (@{event='acquired'; utc=[DateTime]::UtcNow.ToString('o'); pid=$PID; command=$TaskCommand} | ConvertTo-Json -Compress)
        Write-Output "p-verify joined shared build queue at $([DateTime]::UtcNow.ToString('o'))"
        & py -3 -B "$PSScriptRoot/p-verify.py" wait 30 @TaskCommand
        $taskExit = $LASTEXITCODE
    }
} finally {
    if ($taskAcquired) {
        $taskMutex.ReleaseMutex()
        Add-Content -LiteralPath $taskAudit -Encoding utf8 -Value (@{event='released'; utc=[DateTime]::UtcNow.ToString('o'); pid=$PID; seconds=$taskClock.Elapsed.TotalSeconds} | ConvertTo-Json -Compress)
    }
    $taskMutex.Dispose()
}
exit $taskExit
