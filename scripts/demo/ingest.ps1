# Demo stack: a feed poller that loses its connection after a few seconds and exits with 1, so
# managed restart has something to do.
function Err($message) { [Console]::Error.WriteLine($message) }
Write-Output '[ingest] connecting to feed sim...'
Start-Sleep -Milliseconds 600
Write-Output '[ingest] subscribed 48 symbols'
$symbols = 'ES', 'NQ', 'CL', 'GC'
$ticks = Get-Random -Minimum 6 -Maximum 12
for ($i = 0; $i -lt $ticks; $i++) {
    Start-Sleep -Milliseconds 700
    Write-Output ("[ingest] tick {0} {1:N2}, lag {2} ms" -f (Get-Random $symbols), (4000 + (Get-Random -Maximum 1500)), (Get-Random -Minimum 4 -Maximum 40))
}
Err 'Traceback (most recent call last):'
Err '  File "C:\projects\acme-markets\ingest\poll.py", line 88, in <module>'
Err '    main()'
Err '  File "C:\projects\acme-markets\ingest\poll.py", line 61, in main'
Err '    session = feed.connect(timeout=5)'
Err 'ConnectionResetError: [WinError 10054] An existing connection was forcibly closed by the remote host'
exit 1
