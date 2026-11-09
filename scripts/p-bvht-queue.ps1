param([int]$WaitSeconds = 6000)
$taskMutex = [System.Threading.Mutex]::new($false, 'Global\stillkit-cargo-one-at-a-time')
$taskAcquired = $false
$taskExit = 3
$taskAudit = 'D:/Coding/renderlab-work/p-bvht/mutex.jsonl'
$taskClock = [System.Diagnostics.Stopwatch]::new()
try {
    try { $taskAcquired = $taskMutex.WaitOne([TimeSpan]::FromSeconds($WaitSeconds)) }
    catch [System.Threading.AbandonedMutexException] { $taskAcquired = $true }
    if ($taskAcquired) {
        $taskClock.Start()
        Add-Content -LiteralPath $taskAudit -Value (@{event='acquired'; utc=[DateTime]::UtcNow.ToString('o'); pid=$PID} | ConvertTo-Json -Compress)
        & py -3 -B "$PSScriptRoot/p-bvht.py" lease
        $taskExit = $LASTEXITCODE
    }
} finally {
    if ($taskAcquired) {
        $taskMutex.ReleaseMutex()
        Add-Content -LiteralPath $taskAudit -Value (@{event='released'; utc=[DateTime]::UtcNow.ToString('o'); pid=$PID; seconds=$taskClock.Elapsed.TotalSeconds} | ConvertTo-Json -Compress)
    }
    $taskMutex.Dispose()
}
exit $taskExit
