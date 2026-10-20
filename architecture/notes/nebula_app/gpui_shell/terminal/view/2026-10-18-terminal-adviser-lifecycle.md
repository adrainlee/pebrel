# Terminal adviser context and request ownership

## Status

Implemented locally; pending maintainer review.

## Context

The GPUI terminal needs manual command explanations, selected-log analysis and
optional analysis after failed shell commands. Terminal output can contain private
data and untrusted instructions. Replies can arrive after a pane, command or
analysis has changed. The existing legacy failure bar does not own GPUI focus or
view lifetime, so attaching its event bridge to the new panel would mix owners.

## Evidence

- [Terminal reading metadata](../../../../../../nebula_terminal/src/term/reading.rs)
  supplies completed command identities, exit codes and validated output spans.
  Screen keywords alone cannot identify which command produced an error.
- [Provider selection](../../../../../../nebula_app/src/ai_providers.rs) already
  owns the active provider and secure credential lookup.
- [Adviser interaction tests](../../../../../../nebula_app/src/gpui_shell/terminal/view/assistant/tests.rs)
  exercise actual controls and check the fake PTY input channel, rather than
  treating callback invocation as proof that no command was executed.

## Decision

The pane owns its panel, focus, opt-in monitoring flag, request generation and
GPUI task. Closing a panel, switching analysis modes or exiting a pane invalidates
pending results. Manual and automatic request snapshots/results remain separate;
an automatic reply cannot replace an answer being read manually. Disabling
monitoring cancels automatic analysis without canceling a manual request.
The panel's captured failure is separate from the last submitted request, so
opening another failure cannot relabel a cached answer. Pointer down/up/move
events stop at the panel; a middle click must not return text input to the PTY.

Only recorded command completion can trigger automatic analysis. Monitoring is
off initially and is not persisted. Completed identities are deduplicated and
requests have a per-pane cooldown. Unsupported shells retain manual analysis.
Output capture uses validated spans and a bounded suffix; neighboring commands
are excluded. No periodic whole-screen upload or failure inference is added.

The [shared analysis capability](../../../../../../nebula_app/src/ai_assistant/analysis.rs)
owns context limits, token-shaped-string masking, model instructions and reply
validation. Blocking provider lookup and HTTP run in a background task, using
only the active enabled Settings provider. The legacy environment-key fallback
does not apply. Four process-wide request slots and a 30-second HTTP timeout
bound work; canceling a view task does not interrupt an already-running blocking
HTTP call or retract context already sent to a provider.
Provider analysis and connection tests share a transport policy that rejects
redirects. A loopback regression reproduced the HTTP library forwarding the
Anthropic key header on a 302 to a second endpoint; stripping only Authorization
does not protect provider-specific credentials. Context truncation preserves
enough of a preceding token run to retain its redaction classification. The
directory sent is explicitly the current directory, not historical execution cwd.

Suggestions remain text. Inserting one requires an explicit click, the same
terminal input epoch and an empty recognized prompt outside interactive modes.
Insertion targets only the owning pane and includes no Enter. Commands containing
control characters are rejected. Local danger checks supplement the model flag;
neither is a complete assessment of command safety or secret detection.

The palette action type lives alongside its dispatcher in `workspace/palette.rs`,
so extending adviser entry points does not grow the legacy workspace root.

## Rejected alternatives

- Reusing legacy window-loop events: it would give GPUI requests an unrelated
  lifetime and require a second result-routing bridge.
- Inferring failures from words such as “error” or from output silence: logs,
  interactive programs and successful commands can all contain those signals.
- Falling back to an environment API key: context could go to a provider the user
  did not select for the adviser.
- Automatically executing a correction or broadcasting insertion: model output
  is untrusted and belongs to one command context in one pane.

## Consequences

The feature adds no persistent format or dependency. Canceled HTTP may still
consume provider quota until completion/timeout. Bounded snapshots can omit older
log context, and invalidated/oversized cells require manual log selection.
Shell completion integration is required for automatic diagnosis, but not for
manual explanations or pasted logs. Masking long token-like strings is heuristic;
users must review other sensitive context before sending it.

## Validation

The interaction tests cover focus/input isolation, real Analyze/Copy/Insert
controls, cancellation, retry, narrow-panel bounds, stale generations, changed
input epochs and command-boundary capture. Loopback HTTP tests in
[analysis tests](../../../../../../nebula_app/src/ai_assistant/analysis/tests.rs)
exercise protocol routing, authentication headers, task prompt/token budget,
custom full URLs, malformed/oversized replies and HTTP errors without touching
the user's credentials or real model endpoints. Native visual acceptance and
live model quality are separate checks, not established by these tests.

## Supersedes

None. The legacy failure-bar behavior remains a compatibility path.

## Revisit when

Monitoring must persist across panes, command provenance becomes richer, a
provider needs streaming/cancelable transport, or bounded HTTP blocks unrelated
background work in measured workloads.
