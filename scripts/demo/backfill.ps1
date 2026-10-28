# Demo stack: a nightly job that runs for a while and exits cleanly.
Write-Output '[backfill] rolling 1 day for 48 symbols'
$symbols = 'ES', 'NQ', 'CL', 'GC', 'ZN', '6E', 'RTY', 'YM'
for ($i = 1; $i -le 12; $i++) {
    Start-Sleep -Milliseconds 800
    Write-Output ("[backfill] {0} {1}/48, {2:N0} bars" -f (Get-Random $symbols), ($i * 4), (Get-Random -Minimum 12000 -Maximum 22000))
}
Write-Output '[backfill] done: 874,112 bars in 13 m 52 s'
