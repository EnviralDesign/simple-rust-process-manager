# Demo stack: Storybook.
Write-Output '@storybook/cli v8.3.0'
Write-Output 'Storybook 8.3.0 for react-vite started'
Write-Output '  Local: http://localhost:6006/'
while ($true) {
    Start-Sleep -Seconds (Get-Random -Minimum 5 -Maximum 15)
    Write-Output ("[storybook] rebuilt preview in {0} ms" -f (Get-Random -Minimum 120 -Maximum 600))
}
