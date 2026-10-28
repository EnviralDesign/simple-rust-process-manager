# Demo stack: a nightly job that runs for a while and exits cleanly. It runs in a terminal
# (terminal_mode "Auto"), so it writes colour and redraws its progress line in place, as tools
# like pip, cargo and Rich do. (Characters are spelled as codes: PowerShell 5.1 reads scripts
# without a BOM as ANSI.)
$e = [char]27
$heavy = [string][char]0x2501
$check = [char]0x2713
$dot = [char]0x00B7
$symbols = 'ES', 'NQ', 'CL', 'GC', 'ZN', '6E', 'RTY', 'YM'
Write-Output "$e[1mbackfill$e[22m rolling $e[36m1 day$e[39m for 48 symbols"
for ($i = 1; $i -le 12; $i++) {
    $symbol = Get-Random $symbols
    for ($step = 0; $step -le 20; $step++) {
        $bar = "$e[35m" + ($heavy * $step) + "$e[90m" + ($heavy * (20 - $step)) + "$e[39m"
        Write-Host -NoNewline ("`r  {0,-4} {1} {2,3}%" -f $symbol, $bar, ($step * 5))
        Start-Sleep -Milliseconds 30
    }
    $bars = Get-Random -Minimum 12000 -Maximum 22000
    Write-Host ("`r  $e[32m{0}$e[39m {1,-4} {2}/48 {3} {4:N0} bars{5}" -f $check, $symbol, ($i * 4), $dot, $bars, (' ' * 12))
}
Write-Output "$e[32mdone$e[39m: 874,112 bars in 13 m 52 s"
