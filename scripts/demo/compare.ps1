# Demo stack: a small tool started by hand.
Write-Output 'compare-tool listening on http://127.0.0.1:8765'
while ($true) {
    Start-Sleep -Seconds (Get-Random -Minimum 2 -Maximum 6)
    Write-Output ("GET /compare?left=ES&right=NQ 200 {0} ms" -f (Get-Random -Minimum 8 -Maximum 80))
}
