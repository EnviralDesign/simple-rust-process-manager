$ErrorActionPreference = "Stop"
Push-Location (Join-Path $PSScriptRoot "..")
try {
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Windows release build failed" }
    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed" }
    $version = $metadata.packages[0].version
    $executable = Join-Path $metadata.target_directory "release/simple-rust-process-manager.exe"
    $architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
    New-Item -ItemType Directory -Force -Path "dist" | Out-Null
    $archive = "dist/simple-rust-process-manager-$version-windows-$architecture.zip"
    Compress-Archive -Path $executable, "README.md" -DestinationPath $archive -Force
    Write-Host "Archive: $archive"
}
finally {
    Pop-Location
}
