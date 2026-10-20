# Use the application's actual arguments after PowerShell has expanded variables
# and arrays. Console writes stay outside PowerShell's success-output pipeline.
function global:Invoke-PebrelConnection {
    param([string]$Program, [object[]]$Arguments)
    $resetInput = $null
    if ($Program -eq 'ssh' -or $Program -eq 'ssh.exe') {
        $esc = [char]27
        # Commit pending output first. Visit both screens to discard both Kitty
        # stacks, even if SSH already left the alternate screen before failing.
        # Return to the primary screen without erasing its contents or history.
        # Keep ConPTY's Win32 input (9001) and the shell's paste mode (2004).
        $resetInput = "$esc[?2026l$esc[?1049h$esc[<65535u$esc[?1049l$esc[<65535u" +
            "$esc[?1000l$esc[?1002l$esc[?1003l$esc[?1004l$esc[?1005l$esc[?1006l"
    }
    $token = $global:PebrelShellToken
    $owner = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($token))
    $words = @($Program) + @($Arguments | ForEach-Object { [string]$_ })
    $payload = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($owner + "`n" + ($words -join [char]0)))
    [Console]::Write("$([char]27)]1337;SetUserVar=pebrel_shell=$token$([char]7)")
    [Console]::Write("$([char]27)]1337;SetUserVar=pebrel_connection=$payload$([char]7)")
    try {
        if ($resetInput) { [Console]::Write($resetInput) }
        $application = Get-Command -Name $Program -CommandType Application -ErrorAction Stop | Select-Object -First 1
        & $application @Arguments
    } finally {
        # Console writes do not enter the success pipeline or overwrite LASTEXITCODE.
        # This also runs for a failed launch and SSH's connection-loss exit (255).
        if ($resetInput) { [Console]::Write($resetInput) }
        [Console]::Write("$([char]27)]1337;SetUserVar=pebrel_shell=$token$([char]7)")
    }
}

# A user's function/alias keeps precedence. Fully qualified executable paths
# also remain available as an explicit way to invoke the application directly.
if ((Get-Command ssh -ErrorAction SilentlyContinue).CommandType -eq 'Application') {
    function global:ssh { Invoke-PebrelConnection 'ssh' $args }
}
if ((Get-Command ssh.exe -ErrorAction SilentlyContinue).CommandType -eq 'Application') {
    function global:ssh.exe { Invoke-PebrelConnection 'ssh' $args }
}
if ((Get-Command wsl -ErrorAction SilentlyContinue).CommandType -eq 'Application') {
    function global:wsl { Invoke-PebrelConnection 'wsl' $args }
}
if ((Get-Command wsl.exe -ErrorAction SilentlyContinue).CommandType -eq 'Application') {
    function global:wsl.exe { Invoke-PebrelConnection 'wsl' $args }
}
