param(
    [ValidateSet("wgpu-default", "wgpu-dx12", "wgpu-vulkan")]
    [string]$Renderer = "wgpu-vulkan",

    [ValidateSet("auto-vsync", "auto-no-vsync")]
    [string]$PresentMode = "auto-no-vsync",

    [ValidateSet("off", "startup", "continuous")]
    [string]$CaptionSync = "startup",

    [string]$Session = "",

    [switch]$Diagnostics,
    [switch]$RunAndReturn,
    [switch]$NoVsync
)

if ([string]::IsNullOrWhiteSpace($Session)) {
    $Session = "$(Get-Date -Format 'yyyyMMdd-HHmmss')-$Renderer-$PresentMode-$CaptionSync"
}

$effectiveVsync = -not ($NoVsync -or $PresentMode -eq "auto-no-vsync")

$previous = @{
    PM_USE_RUNTIME_TOGGLES = $env:PM_USE_RUNTIME_TOGGLES
    PM_RENDERER = $env:PM_RENDERER
    PM_PRESENT_MODE = $env:PM_PRESENT_MODE
    PM_CAPTION_SYNC = $env:PM_CAPTION_SYNC
    PM_DIAG_SESSION = $env:PM_DIAG_SESSION
    PM_DIAGNOSTICS = $env:PM_DIAGNOSTICS
    PM_RUN_AND_RETURN = $env:PM_RUN_AND_RETURN
    PM_VSYNC = $env:PM_VSYNC
}

Write-Host "Renderer: $Renderer"
Write-Host "Present mode: $PresentMode"
Write-Host "Caption sync: $CaptionSync"
Write-Host "Session: $Session"
Write-Host "Diagnostics: $($Diagnostics.IsPresent)"
Write-Host "Run and return: $($RunAndReturn.IsPresent)"
Write-Host "Vsync: $effectiveVsync"

try {
    $env:PM_USE_RUNTIME_TOGGLES = "1"
    $env:PM_RENDERER = $Renderer
    $env:PM_PRESENT_MODE = $PresentMode
    $env:PM_CAPTION_SYNC = $CaptionSync
    $env:PM_DIAG_SESSION = $Session
    $env:PM_DIAGNOSTICS = if ($Diagnostics) { "1" } else { "0" }
    $env:PM_RUN_AND_RETURN = if ($RunAndReturn) { "1" } else { "0" }
    $env:PM_VSYNC = if ($effectiveVsync) { "1" } else { "0" }

    cargo run
}
finally {
    foreach ($key in $previous.Keys) {
        if ($null -eq $previous[$key]) {
            Remove-Item "Env:$key" -ErrorAction SilentlyContinue
        }
        else {
            Set-Item "Env:$key" $previous[$key]
        }
    }
}
