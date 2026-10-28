# Demo stack: a Vite dev server.
Write-Output ''
Write-Output '  VITE v5.4.2  ready in 612 ms'
Write-Output ''
Write-Output '  ->  Local:   http://localhost:5173/'
Write-Output '  ->  Network: use --host to expose'
$files = '/src/routes/chart.tsx', '/src/components/Watchlist.tsx, /src/styles/app.css', '/src/components/DepthLadder.tsx'
while ($true) {
    Start-Sleep -Seconds (Get-Random -Minimum 4 -Maximum 12)
    $time = Get-Date -Format 'h:mm:ss tt'
    if ((Get-Random -Maximum 4) -eq 0) {
        Write-Output "$time [vite] page reload src/lib/api.ts"
    } else {
        Write-Output "$time [vite] hmr update $(Get-Random $files)"
    }
}
