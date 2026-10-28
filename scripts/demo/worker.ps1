# Demo stack: a queue worker that burns a little CPU per job.
Write-Output '[worker] connected to redis://127.0.0.1:6379/0'
Write-Output '[worker] 4 threads, listening on queue default'
$symbols = 'ES', 'NQ', 'CL', 'GC', 'ZN', '6E', 'RTY', 'YM'
$rollups = '1m -> 5m', '5m -> 15m', '15m -> 1h'
while ($true) {
    $job = Get-Random -Minimum 8800 -Maximum 9900
    $until = (Get-Date).AddMilliseconds((Get-Random -Minimum 60 -Maximum 400))
    $x = 0
    while ((Get-Date) -lt $until) { $x = ($x + 1) % 7919 }
    $roll = Get-Random -Maximum 100
    if ($roll -lt 5) {
        [Console]::Error.WriteLine("[worker] WARNING retrying job $job (attempt 2/5): lock held by another worker")
    } elseif ($roll -lt 20) {
        Write-Output ("[worker] queue default, depth {0}, {1} jobs/min" -f (Get-Random -Maximum 7), (Get-Random -Minimum 40 -Maximum 120))
    } else {
        Write-Output ("[worker] job {0} bars.rollup {1} {2} done in {3} ms" -f $job, (Get-Random $symbols), (Get-Random $rollups), (Get-Random -Minimum 60 -Maximum 900))
    }
    Start-Sleep -Milliseconds (Get-Random -Minimum 300 -Maximum 1500)
}
