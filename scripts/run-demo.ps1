<#
.SYNOPSIS
Runs Process Manager against the demo stack in scripts/demo, or saves a screenshot of it.

.EXAMPLE
./scripts/run-demo.ps1
./scripts/run-demo.ps1 -Screenshot images/overview.png -Window 1440x900 -Open overview
./scripts/run-demo.ps1 -Screenshot .tmp/small.png -Window 480x560 -Open settings:ingest

-Open takes overview, stack, new, settings, activity or palette, optionally with :<process>.
The demo lives in .tmp/demo (an executable beside its own processes.json), so your own
processes.json is never touched.
#>
param(
    [string]$Screenshot,
    [string]$Window = '1440x900',
    [string]$Open,
    [switch]$Release
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$profileName = if ($Release) { 'release' } else { 'debug' }
Push-Location $root
try {
    if ($Release) { cargo build --release } else { cargo build }
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally {
    Pop-Location
}

$demo = Join-Path $root '.tmp\demo'
if (Test-Path $demo) { Remove-Item -Recurse -Force $demo }
New-Item -ItemType Directory -Force (Join-Path $demo 'demo') | Out-Null
Copy-Item (Join-Path $root "target\$profileName\simple-rust-process-manager.exe") (Join-Path $demo 'pm.exe')
Copy-Item (Join-Path $PSScriptRoot 'demo\*') (Join-Path $demo 'demo') -Exclude 'processes.json'
Copy-Item (Join-Path $PSScriptRoot 'demo\processes.json') (Join-Path $demo 'processes.json')

$env:PM_WINDOW = $Window
$env:PM_OPEN = $Open
if ($Screenshot) {
    $env:PM_SCREENSHOT = [System.IO.Path]::GetFullPath((Join-Path $root $Screenshot))
    New-Item -ItemType Directory -Force (Split-Path -Parent $env:PM_SCREENSHOT) | Out-Null
    $process = Start-Process -FilePath (Join-Path $demo 'pm.exe') -PassThru -WorkingDirectory $demo
    if (-not $process.WaitForExit(60000)) {
        Stop-Process -Id $process.Id -Force
        throw 'Timed out waiting for the screenshot'
    }
    Write-Output "Saved $env:PM_SCREENSHOT"
} else {
    Remove-Item Env:PM_SCREENSHOT -ErrorAction SilentlyContinue
    & (Join-Path $demo 'pm.exe')
}
