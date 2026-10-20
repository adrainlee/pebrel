# Approved log queries on the current pane

## Status

Implemented; pending maintainer review.

## Context

Copying terminal output into the assistant adds work during an incident. The user
requested queries through the current connection, with separate approval for each
query. The existing assistant only proposed commands for manual insertion.

## Evidence

`ssh_session/transcript.rs` already exposes a weak, pane-owned capability to the
exact authenticated transport. Its scope ends with the pane's shell channel.
`runtime_exec.rs` owns isolated local/WSL processes with output limits and process
group cleanup. Neither requires writing input to an interactive terminal.

## Decision

- The model proposes one typed diagnostic query, separately from its suggested
  command string. Supported queries read a file tail, list one directory, read
  a bounded systemd journal window, or read a bounded Docker log window.
- Validation and argument construction have one authority in `ai_assistant/query`.
  Unknown fields/tools, control characters, relative paths, option-like service
  names and unbounded windows are rejected. Shell operators in paths remain literal
  data. There is no arbitrary-command, elevation or write tool.
- GPUI captures the query source when requesting advice. An approval control shows
  the source and query parameters, and explicitly includes sending the result to
  the active model. A click consumes that proposal's generation; it cannot approve
  a newer proposal, another pane, a changed cwd/scope or a replacement connection.
- SSH uses a fresh non-PTY channel on the captured transport, with the original
  authenticated account. It does not reconnect, search the connection pool, inherit
  an interactive shell's exports/elevation, or interrupt foreground terminal work.
- Local/WSL reads reuse `PaneExecContext` and the existing child-process owner.
  Windows file queries use a fixed encoded PowerShell program with literal paths;
  Windows supports file-tail and directory queries. Unverified typed remote shells
  do not fall back to local execution. SSH queries currently require Unix tools.
- Queries share the assistant's four-worker admission limit. Each query has a
  ten-second deadline and bounded capture. The model receives a redacted excerpt
  of at most 12 KiB, with exit/truncation evidence. File reads retain an additional
  4 KiB before the excerpt so credential labels near its boundary can be redacted.
  A query failure is visible and
  never authorizes an automatic retry. Each next query requires another click.
- The UI owns a cancellation guard. Cancellation, collapse and destruction revoke
  it; SSH scope closure independently cancels reads. A late result cannot restart
  the model loop after the owning generation or source changed.

## Rejected alternatives

- Sending model-generated command strings to the PTY: risks interacting with a
  foreground program, changes shell state and makes result boundaries ambiguous.
- Treating a model's `danger=false` as a read-only policy: this is untrusted output,
  not an enforcement boundary.
- Looking up the SSH destination again when executing: a reconnect or a second
  pane can bind the approval to the wrong transport.
- Using one blanket session approval: the user explicitly chose per-query approval.
- Installing a remote agent or adding a new process executor: the existing SSH and
  process owners already provide the required lifecycle.

## Consequences

The assistant can collect evidence directly without gaining unrestricted command
execution. Permissions remain those of the SSH login/local process; protected logs
can fail with permission errors and require the user's manual action. Read tools
are not an OS sandbox, and redaction is best effort. The SSH server's command
environment and Docker daemon configuration still determine where those tools run.
Directory listings and logs can be truncated; analysis must preserve that uncertainty.

## Validation

Tests cover typed-query validation and injection boundaries, actual Windows file
tail reads and redaction, rendered approval/cancel controls, fresh approval for each
query, stale-source rejection and no PTY writes. Loopback SSH tests cover reusing the
transport, channel cleanup, cancellation and source revocation. Native process tests
cover cancellation. Product compilation, architecture and i18n checks are required.
On Windows, the targeted run passed 61 application tests (one existing SSH stress
test remained ignored), product check/build, 24 i18n tests, settings and line-budget
tests, and architecture validation. The checker suite ran 55 tests with three skips.
Live customer servers, remote Windows and visual approval are not claimed by these
tests; no customer host or configured model is contacted during validation.

## Supersedes

Extends the manual-advice boundary in
`../gpui_shell/terminal/view/2026-10-18-terminal-adviser-lifecycle.md`;
suggested command strings retain their manual-only behavior.

## Revisit when

More query types, remote Windows support, long-running monitoring, per-session
approval, or environment changes inside the interactive shell become requirements.
