//! External SSH owns input modes only until the PowerShell wrapper returns.
#![cfg(windows)]

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use nebula_terminal::Term;
use nebula_terminal::event::VoidListener;
use nebula_terminal::event_loop::StreamProcessor;
use nebula_terminal::index::{Column, Line};
use nebula_terminal::term::{Config, TermMode, test::TermSize};

struct Script(std::path::PathBuf);

impl Script {
    fn new(body: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "pebrel-ssh-recovery-{}-{}.ps1",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, format!("\u{feff}{body}")).unwrap();
        Self(path)
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run_script(checks: &str) -> Vec<u8> {
    let script = Script::new(&format!(
        "$ErrorActionPreference = 'Stop'\n\
         $global:PebrelShellToken = 'dGVzdC1vd25lcg=='\n{}\n{}",
        include_str!("../src/tty/connection.ps1"),
        checks
    ));
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script.0)
        .output()
        .expect("Windows PowerShell must execute the production wrapper");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    output.stdout
}

fn terminal() -> Term<VoidListener> {
    Term::new(
        Config { kitty_keyboard: true, ..Config::default() },
        &TermSize::new(100, 24),
        VoidListener,
    )
}

fn stale_modes() -> TermMode {
    TermMode::KITTY_KEYBOARD_PROTOCOL
        | TermMode::MOUSE_MODE
        | TermMode::SGR_MOUSE
        | TermMode::UTF8_MOUSE
        | TermMode::FOCUS_IN_OUT
        | TermMode::ALT_SCREEN
}

#[test]
fn ssh_wrapper_recovers_before_launch_and_after_exit_without_prompt_markers() {
    // Replace executable discovery, not the production SSH wrapper. No network.
    let output = run_script(
        r#"
function global:Get-Command {
    param($Name, $CommandType, $ErrorAction)
    Microsoft.PowerShell.Core\Get-Command powershell.exe -CommandType Application
}
foreach ($program in @('ssh', 'ssh.exe')) {
    foreach ($code in @(0, 255)) {
        [Console]::Write('BEGIN')
        Invoke-PebrelConnection $program @('-NoProfile', '-NonInteractive', '-Command', "[Console]::Write('CHILD'); exit $code")
        if ($LASTEXITCODE -ne $code) { throw 'Exit status changed' }
        [Console]::Write('END')
    }
}
"#,
    );
    assert!(!output.windows(6).any(|w| w == b"\x1b]133;"));
    let text = String::from_utf8(output).unwrap();
    let mut count = 0;
    for cycle in text.split("BEGIN").skip(1) {
        let (before, after) = cycle.split_once("CHILD").expect("child ran");
        let after = after.split_once("END").unwrap().0;
        for recovery in [before.as_bytes(), after.as_bytes()] {
            for abandoned in [
                b"\x1b[>1u\x1b[?1049h\x1b[>7u".as_slice(),
                b"\x1b[>1u\x1b[?1049h\x1b[>7u\x1b[?1049l",
            ] {
                for split in 0..=recovery.len() {
                    let mut term = terminal();
                    let mut stream = StreamProcessor::default();
                    stream.feed(&mut term, &VoidListener, b"\x1b[?9001h\x1b[?2004h");
                    stream.feed(&mut term, &VoidListener, b"saved history\r\n");
                    stream.feed(&mut term, &VoidListener, abandoned);
                    stream.feed(
                        &mut term,
                        &VoidListener,
                        b"\x1b[?2026h\x1b[?1003h\x1b[?1006h\x1b[?1004h",
                    );
                    stream.feed(&mut term, &VoidListener, &recovery[..split]);
                    stream.feed(&mut term, &VoidListener, &recovery[split..]);
                    assert_eq!(stream.sync_bytes_count(), 0, "unfinished sync, split={split}");
                    assert!(!term.mode().intersects(stale_modes()), "stale {:?}", term.mode());
                    assert_eq!(term.grid()[Line(0)][Column(0)].c, 's');
                    assert!(
                        term.mode()
                            .contains(TermMode::WIN32_INPUT_MODE | TermMode::BRACKETED_PASTE)
                    );
                    // Neither screen may resurrect a previous connection's stack.
                    for sequence in
                        [b"\x1b[?1049h".as_slice(), b"\x1b[<u", b"\x1b[?1049l", b"\x1b[<u"]
                    {
                        stream.feed(&mut term, &VoidListener, sequence);
                        assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
                    }
                    assert!(!term.mode().intersects(stale_modes()));
                    stream.feed(&mut term, &VoidListener, b"\x1b[>7u\x1b[?1003h\x1b[?1006h");
                    assert!(term.mode().contains(TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE));
                    assert!(term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
                }
            }
        }
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn ssh_preserves_arguments_pipeline_exit_status_and_user_overrides() {
    let child = Script::new("[Console]::Write($args[0]); exit 7");
    let output = run_script(&format!(
        r#"
function global:Get-Command {{
    param($Name, $CommandType, $ErrorAction)
    Microsoft.PowerShell.Core\Get-Command powershell.exe -CommandType Application
}}
$argument = "space ' quote; pipe | dollar `$"
$capture = New-Object System.IO.StringWriter
$previous = [Console]::Out
try {{
    [Console]::SetOut($capture)
    $result = Invoke-PebrelConnection 'ssh' @('-NoProfile', '-NonInteractive', '-File', '{}', $argument)
    if ($LASTEXITCODE -ne 7) {{ throw 'SSH exit status changed' }}
}} finally {{ [Console]::SetOut($previous) }}
if ([string]$result -ne $argument) {{ throw 'Arguments or pipeline contaminated' }}
Remove-Item Function:\Get-Command
function global:ssh {{ 'USER_FUNCTION' }}
function global:Test-SshAlias {{ 'USER_ALIAS' }}
Set-Alias -Name ssh.exe -Value Test-SshAlias -Scope Global
{}
if ((ssh) -ne 'USER_FUNCTION' -or (ssh.exe) -ne 'USER_ALIAS') {{ throw 'User override replaced' }}
[Console]::Write('CONTRACTS_OK')
"#,
        child.0.display().to_string().replace('\'', "''"),
        include_str!("../src/tty/connection.ps1")
    ));
    assert_eq!(output, b"CONTRACTS_OK");
}

#[test]
fn ssh_cleanup_runs_on_launch_error_and_does_not_change_other_connections() {
    let output = run_script(
        r#"
function global:Get-Command {
    param($Name, $CommandType, $ErrorAction)
    if ($Name -eq 'ssh') { return 'pebrel-test-missing-application.exe' }
    Microsoft.PowerShell.Core\Get-Command $Name -CommandType Application
}

try { Invoke-PebrelConnection 'ssh' @(); throw 'Expected launch failure' }
catch [System.Management.Automation.CommandNotFoundException] { }
[Console]::Write('OTHER')
Invoke-PebrelConnection 'cmd.exe' @('/d', '/c', 'exit 7')
if ($LASTEXITCODE -ne 7) { throw 'Other exit status changed' }
"#,
    );
    let text = String::from_utf8(output).unwrap();
    let (ssh, other) = text.split_once("OTHER").unwrap();
    assert_eq!(ssh.matches("\x1b[?2026l").count(), 2, "before and finally");
    assert!(!other.contains("\x1b[?2026l"), "non-SSH connections retain their modes");
}

#[test]
fn reconnect_restores_native_and_translated_readers_through_real_conpty() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    use nebula_terminal::event::WindowSize;
    use nebula_terminal::tty::{self, EventedReadWrite, Options, Shell};

    let child = Script::new(include_str!("fixtures/ssh_input_reader.ps1"));
    let parent = Script::new(&format!(
        r#"
$ErrorActionPreference = 'Stop'
$global:PebrelShellToken = 'dGVzdC1vd25lcg=='
function global:Get-Command {{
    param($Name, $CommandType, $ErrorAction)
    Microsoft.PowerShell.Core\Get-Command powershell.exe -CommandType Application
}}
{}
foreach ($reader in @('native', 'bytes')) {{
    ssh -NoProfile -ExecutionPolicy Bypass -File '{}' dirty $reader
    if ($LASTEXITCODE -ne 255) {{ throw 'Lost SSH exit status' }}
    ssh.exe -NoProfile -ExecutionPolicy Bypass -File '{}' $reader
}}
[Console]::Write('RECOVERY_DONE')
"#,
        include_str!("../src/tty/connection.ps1"),
        child.0.display().to_string().replace('\'', "''"),
        child.0.display().to_string().replace('\'', "''")
    ));
    let options = Options {
        shell: Some(Shell::new(
            "powershell.exe".into(),
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                parent.0.to_string_lossy().into_owned(),
            ],
        )),
        ..Options::default()
    };
    let size = WindowSize { num_lines: 24, num_cols: 100, cell_width: 8, cell_height: 16 };
    let mut pty = tty::new(&options, size, 0).expect("create real ConPTY");
    let mut term = terminal();
    let mut stream = StreamProcessor::default();
    let mut output = Vec::new();
    let mut pending = Vec::new();
    let mut dirty_count = 0;
    let mut reader_count = 0;
    let mut phase = 0;
    let handshakes: [&[u8]; 6] = [
        b"DIRTY_READY_native",
        b"READER_READY_native",
        b"FIRST_native_27",
        b"DIRTY_READY_bytes",
        b"READER_READY_bytes",
        b"FIRST_bytes_27",
    ];
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        let mut chunk = [0; 8192];
        match pty.reader().read(&mut chunk) {
            Ok(0) => std::thread::sleep(Duration::from_millis(10)),
            Ok(count) => {
                // Feed byte by byte so mode assertions occur at the handshake,
                // independent of pipe chunk boundaries and later child output.
                for byte in &chunk[..count] {
                    stream.feed(&mut term, &VoidListener, std::slice::from_ref(byte));
                    output.push(*byte);
                    pending.push(*byte);
                    if pending.ends_with(b"\x1b[6n") {
                        pty.writer().write_all(b"\x1b[1;1R").unwrap();
                    }
                    let expected =
                        handshakes.get(phase).is_some_and(|marker| pending.ends_with(marker));
                    if expected && phase % 3 == 0 {
                        assert!(term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
                        // In-box ConPTY consumes mouse negotiation itself;
                        // bundled OpenConsole forwards it to the emulator.
                        if term.mode().contains(TermMode::WIN32_INPUT_MODE) {
                            assert!(
                                term.mode().contains(TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE)
                            );
                        }
                        pty.writer().write_all(b"\r").unwrap();
                        dirty_count += 1;
                    }
                    if expected && phase % 3 == 1 {
                        // OpenConsole may renegotiate focus for its own console
                        // reader; SSH's mouse and keyboard modes must be gone.
                        assert!(
                            !term.mode().intersects(stale_modes() - TermMode::FOCUS_IN_OUT),
                            "{:?}",
                            term.mode()
                        );
                        assert_eq!(stream.sync_bytes_count(), 0);
                        // Same records emitted by the product encoder, with
                        // the legacy fallback for the in-box ConPTY host.
                        let win32 = term.mode().contains(TermMode::WIN32_INPUT_MODE);
                        eprintln!("reader handshake: Win32 input={win32}");
                        let keys: &[u8] =
                            if win32 { b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_" } else { b"\x1b" };
                        pty.writer().write_all(keys).unwrap();
                        reader_count += 1;
                    }
                    if expected && phase % 3 == 2 {
                        let keys: &[u8] = if term.mode().contains(TermMode::WIN32_INPUT_MODE) {
                            b"\x1b[67;46;3;1;8;1_\x1b[67;46;3;0;8;1_"
                        } else {
                            b"\x03"
                        };
                        pty.writer().write_all(keys).unwrap();
                    }
                    if expected {
                        phase += 1;
                    }
                    if pending.len() > 128 {
                        pending.drain(..64);
                    }
                }
                if output.windows(13).any(|w| w == b"RECOVERY_DONE") {
                    break;
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            },
            Err(error) => panic!("PTY read: {error}"),
        }
    }
    drop(pty);
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("RECOVERY_DONE"), "timeout or EOF: {text:?}");
    assert!(text.contains("RESULT_native_27_3"), "native reader: {text:?}");
    assert!(text.contains("RESULT_bytes_27_3"), "translated reader: {text:?}");
    assert_eq!((dirty_count, reader_count), (2, 2));
    assert!(!text.contains("\x1b]133;A"), "must not depend on prompt recovery");
}
