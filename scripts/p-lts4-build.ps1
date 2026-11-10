param([string]$Label = 'base')
$ErrorActionPreference = 'Stop'
$taskMutex = [System.Threading.Mutex]::new($false, 'Global\stillkit-cargo-one-at-a-time')
$taskAcquired = $false
try {
    try { $taskAcquired = $taskMutex.WaitOne([TimeSpan]::FromMinutes(15)) }
    catch [System.Threading.AbandonedMutexException] { $taskAcquired = $true }
    if (!$taskAcquired) { throw 'build mutex timeout' }
    & py -3 -B "$PSScriptRoot/p-lts4-build.py" $Label
    $taskCode = $LASTEXITCODE
} finally {
    if ($taskAcquired) { $taskMutex.ReleaseMutex() }
    $taskMutex.Dispose()
}
exit $taskCode
