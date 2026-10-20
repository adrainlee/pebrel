# Recover abandoned input protocols at a shell prompt

Superseded in scope by [PowerShell SSH input lifetime](../tty/2026-10-20-powershell-ssh-input-lifetime.md)
for external PowerShell SSH reconnects; prompt recovery remains a fallback.

## Status

Implemented; real SSH interruption acceptance remains pending.

## Context

An external `ssh` process can exit without its remote foreground application
disabling mouse reporting or popping enhanced keyboard modes. Starting `ssh`
again in that pane retains the emulator state. Reported symptoms were mouse
coordinates at the shell prompt, Esc leaving `u`, and Ctrl+C failing while Enter
still submitted the input line. This is consistent with SGR mouse and CSI-u
input reaching a shell that did not negotiate them; the original live incident
has not been captured at the byte level.

## Evidence

The existing keyboard stack fixes preserve explicit push/set/pop operations.
They do not synthesize cleanup after an external process exits. The built-in
SSH retry creates a new terminal, so fixing that lifecycle would not cover a
command-line SSH client in a surviving local shell.

The new [stream regressions](../../../../nebula_terminal/src/term/prompt/tests.rs)
failed in four recovery cases before the change. Both negative controls passed.

## Decision

Treat OSC 133;A on the primary screen as input ownership returning to the shell.
The terminal clears mouse tracking/encoding flags and both enhanced keyboard
stacks. Keep this rule in `Term`, shared by local PTY and built-in SSH streams.
Notify the host when the mouse cursor needs updating.

The stream commits outstanding synchronized output at the prompt boundary before
applying recovery, so delayed mode enables cannot resurrect abandoned state.
Subsequent bytes retain their wire order and may negotiate fresh protocols.
Command-range recording still rejects markers that arrived in an uncommitted
frame; recovery does not promote those markers into complete command records.

Preserve Win32 native input, bracketed paste, display contents and all other
modes. Ignore recovery on the alternate screen, where a nested prompt may still
belong to a running full-screen application.

## Rejected alternatives

- Reset on any command completion: nested commands can complete inside an active
  application; completion alone does not establish shell input ownership.
- Infer prompts from visible text or idle time: ordinary output and silent TUIs
  cannot prove that the foreground application has ended.
- Reset the entire terminal: this would discard screen state and native console
  input negotiation unrelated to the abandoned application.
- Fix only the built-in SSH retry: external SSH does not use that lifecycle.

## Consequences

Shells must emit OSC 133;A for automatic recovery. Plain prompts without semantic
integration retain manual reset behavior. Shells which themselves need mouse or
enhanced keyboard input must negotiate it after the prompt boundary. An
application left on the alternate screen is deliberately not recovered by this
rule. No process polling, dependencies or persistent state are introduced.

The new work happens at prompt boundaries: fixed flag updates and clearing two
bounded vectors, retaining their allocations. This is not a measured throughput
or latency guarantee.

## Validation

- Six core stream tests cover every two-chunk split of recovery and renewed
  negotiation, both keyboard stacks, native/paste preservation, alternate-screen
  exclusion, silent/plain-text/command-completion exclusion and unfinished sync.
- A product encoder regression checks Ctrl+C and Esc before negotiation, during
  negotiation, after recovery and after a new negotiation, with and without
  Win32 input enabled.
- The core run passed 329 tests with two ignored; two existing PowerShell proxy
  tests could not run because `pwsh.exe` is absent. The pre-existing synchronized
  command-recording regression passes without changing its assertions.
- All 33 product keymap tests, the product compile check and architecture check
  passed. Changed Rust files pass formatting; workspace-wide formatting still
  reports existing newline/style differences outside these changes.
- Native input verification here covers mode preservation and encoded records;
  live native-console and translated-reader acceptance was not performed.
- Real network disconnect/reconnect acceptance is left to the reported workflow.

## Supersedes

None. Explicit keyboard negotiation and stack semantics remain in force.

## Revisit when

A supported shell requires pre-prompt negotiation to survive OSC 133;A, or a
transport-independent, reliable foreground ownership boundary becomes available
for shells without semantic prompt integration.
