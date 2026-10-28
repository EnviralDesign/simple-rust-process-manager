# Demo stack: a uvicorn-style API that logs to stderr, like the real thing.
function Err($message) { [Console]::Error.WriteLine($message) }
Err 'warning: Ignoring dangling temporary directory: `C:\repos\market-data-lake\.venv\Lib\site-packages\~arket_data_lake-0.1.0.dist-info`'
Err "INFO:     Will watch for changes in these directories: ['C:\repos\market-data-lake']"
Err 'INFO:     Uvicorn running on http://127.0.0.1:8010 (Press CTRL+C to quit)'
Err "INFO:     Started reloader process [$PID] using StatReload"
Err "INFO:     Started server process [$($PID + 4)]"
Err 'INFO:     Waiting for application startup.'
Err 'INFO:     Application startup complete.'
$symbols = 'ES', 'NQ', 'CL', 'GC', 'ZN', '6E', 'RTY', 'YM'
$frames = '1m', '5m', '15m', '1h'
while ($true) {
    Start-Sleep -Milliseconds (Get-Random -Minimum 200 -Maximum 1200)
    $symbol = Get-Random $symbols
    $roll = Get-Random -Maximum 100
    if ($roll -lt 4) {
        Err ("WARNING:  slow query: bars_5m scan took {0:N2} s (symbol={1})" -f ((Get-Random -Minimum 120 -Maximum 280) / 100), $symbol)
    } elseif ($roll -lt 5) {
        Err "ERROR:    upstream feed timeout after 5.0 s (symbol=$symbol)"
    } else {
        $route = Get-Random @("GET /v1/bars?symbol=$symbol&tf=$(Get-Random $frames)", "GET /v1/quotes/$symbol", 'GET /v1/health', 'POST /v1/backtests', 'GET /v1/symbols?exchange=CME')
        $code = if ($route.StartsWith('POST')) { '202 Accepted' } else { '200 OK' }
        Err ("INFO:     127.0.0.1:{0} - `"{1} HTTP/1.1`" {2}" -f (Get-Random -Minimum 50100 -Maximum 65400), $route, $code)
    }
}
