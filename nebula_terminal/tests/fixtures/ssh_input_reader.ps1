param([string]$Reader, [string]$Cycle)
$ErrorActionPreference = 'Stop'
# Each handshake is consumed once by the test, even if ConPTY redraws it.
function Write-Probe([string]$Message) {
    [Console]::Write("`r`n$Message`r`n")
}
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InputProbe {
    [DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int id);
    [DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr h, out uint mode);
    [DllImport("kernel32.dll")] public static extern bool SetConsoleMode(IntPtr h, uint mode);
}
'@
$inputHandle = [InputProbe]::GetStdHandle(-10)
$outputHandle = [InputProbe]::GetStdHandle(-11)
[uint32]$oldInput = 0
[uint32]$oldOutput = 0
if (-not [InputProbe]::GetConsoleMode($inputHandle, [ref]$oldInput)) { throw 'No console input' }
if (-not [InputProbe]::GetConsoleMode($outputHandle, [ref]$oldOutput)) { throw 'No console output' }
try {
    [uint32]$inputMode = 0
    if ($Reader -eq 'bytes') { $inputMode = 0x200 }
    if (-not [InputProbe]::SetConsoleMode($inputHandle, $inputMode)) { throw 'Input mode failed' }
    if (-not [InputProbe]::SetConsoleMode($outputHandle, ($oldOutput -bor 4))) { throw 'VT output failed' }
    $esc = [char]27
    if ($Reader -eq 'dirty') {
        [Console]::Write("$esc[>1u$esc[?1049h$esc[>7u$esc[?1003h$esc[?1006h")
        Write-Probe "DIRTY_READY_$Cycle"
        $null = [Console]::ReadKey($true)
        # Leave a synchronized frame unfinished as a lost connection can do.
        [Console]::Write("$esc[?2026h$esc[>7u$esc[?1003h")
        exit 255
    }
    Write-Probe "READER_READY_$Reader"
    if ($Reader -eq 'native') {
        $first = [int][Console]::ReadKey($true).KeyChar
        Write-Probe "FIRST_$($Reader)_$first"
        $second = [int][Console]::ReadKey($true).KeyChar
    } else {
        $inputStream = [Console]::OpenStandardInput()
        $first = $inputStream.ReadByte()
        Write-Probe "FIRST_$($Reader)_$first"
        $second = $inputStream.ReadByte()
    }
    Write-Probe "RESULT_$($Reader)_$($first)_$second"
} finally {
    $null = [InputProbe]::SetConsoleMode($inputHandle, $oldInput)
    $null = [InputProbe]::SetConsoleMode($outputHandle, $oldOutput)
}
