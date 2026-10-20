# Bound external SSH input modes to its PowerShell invocation

## Status

Implemented and locally tested on Windows; real remote disconnect acceptance
remains with the reporting user.

## Context

The reported path is `ssh` in an existing PowerShell pane, followed by disconnect
and another `ssh` invocation in that pane. Mouse coordinates and CSI-u encodings
then reach the remote shell as input. The emulator outlives both SSH processes.
Prompt-only recovery cannot cover absent prompt markers or an abandoned alternate
screen, which deliberately excludes prompt recovery.

## Evidence

The existing [PowerShell wrapper](../../../../nebula_terminal/src/tty/connection.ps1)
already brackets `ssh` and `ssh.exe` with connection identity reports and a
`finally` block. It restored identity but not input protocol state.
Two new lifecycle regressions failed before this change: unfinished synchronized
output retained mode enables, and launch failure emitted no cleanup.

## Decision

Emit standard VT cleanup before application discovery/launch and in `finally`,
only for SSH. End synchronized output first, visit the alternate and primary
screens and pop both bounded Kitty stacks, then disable mouse reporting,
mouse encodings and focus reports. Visiting both screens also clears the inactive
alternate stack when SSH already returned to the primary screen before failing.
The final screen is primary. Preserve its contents/history, Win32 input mode and
bracketed paste. A new foreground application can negotiate again in wire order.

Reuse the existing wrapper and VT parser; add no process polling, new core API,
private protocol or persistent state. Console writes remain outside PowerShell's
success pipeline and do not replace the child exit status. Application discovery
is inside `try` so its failure also restores the parent identity and input state.

## Rejected alternatives

- Prompt-only recovery: no guarantee of OSC 133;A or a primary-screen return.
- Reset on output silence or connection-error text: neither proves process exit.
- Change built-in SSH reconnect: the external client does not use that lifecycle.
- Full terminal reset: unnecessarily discards unrelated state and native input.
- Clear only the active keyboard stack: a later alternate-screen switch can
  resurrect stale flags from the previous connection.

## Consequences

This applies to the installed PowerShell integration's `ssh` / `ssh.exe` wrappers.
User functions/aliases retain precedence; absolute executable paths and shells
launched without integration bypass these wrappers. WSL wrappers are unchanged.
Forced termination of the parent shell cannot execute its `finally` block.
Existing panes must be recreated to load an updated bootstrap.

The extra output occurs only at SSH invocation boundaries. The deliberate screen
round trip can cause a redraw; it does not erase primary-screen history. This is
not a measured throughput claim. ConPTY may independently renegotiate focus after
cleanup; that is distinct from stale remote mouse/keyboard flags.

## Validation

- [Lifecycle tests](../../../../nebula_terminal/tests/ssh_input_recovery.rs) cover
  both command spellings, exit 0/255, launch errors, every cleanup chunk boundary,
  unfinished sync, both screen stacks, preserved native/paste modes, renewed
  negotiation, argument/pipeline/status preservation and user overrides.
- A real ConPTY test runs the production wrappers around a local child that
  negotiates Kitty/mouse/alternate-screen modes and exits 255 without cleanup.
  Two reconnects then use native `ReadKey` and translated stdin byte readers.
  Each receives Esc=27 and Ctrl+C=3 without any OSC 133;A prompt marker.
- The same acceptance passed with bundled OpenConsole (Win32 records negotiated)
  and the local in-box ConPTY (legacy VT input). Handshakes are consumed once
  per phase because the in-box host can repeat visible text during redraw.
- Four new integration tests, the existing connection transport test, and all
  33 product keymap tests passed. Core tests: 329 passed, two ignored, two
  pre-existing proxy tests failed because `pwsh.exe` is absent on this machine.
- No network credentials are used by these tests. They validate the lifecycle
  and readers, not the user's particular SSH server or original packet trace.

## Supersedes

The sufficiency of [prompt recovery](../term/2026-10-20-prompt-input-recovery.md)
for this external PowerShell SSH scenario. Prompt recovery remains a fallback.

## Revisit when

PowerShell invocation semantics change, another supported shell needs equivalent
cleanup, or the host gains a reliable foreground ownership boundary that also
covers direct executable paths without inspecting command text.
