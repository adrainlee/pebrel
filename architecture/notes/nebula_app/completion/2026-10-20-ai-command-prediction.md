# AI completion shares the existing command editor

## Status

Implemented for GPUI terminal command suggestions, with explicit provider opt-in.

## Context

The user requested predictions during typing, informed by scoped command history
and prior assistant recommendations. Tab accepts text and Enter submits it. Model
analysis must wait for at least one completed word to avoid excessive requests.
Existing history/path/Git suggestions must remain usable while a model is slow.

## Evidence

- `completion.rs` already owns shared history and environment-scoped lookup.
- Terminal completion captures prompt input, tracks input/editor revisions and
  owns asynchronous tasks. The native editor may be the authority for the caret.
- `ai_assistant.rs` already owns provider protocols and bounded HTTP responses.
  Reusing it avoids a second credential/protocol implementation.
- Hybrid completion normally uses Tab to open a list; a displayed AI suffix needs
  an explicit acceptance path through the existing edit insertion operation.

## Decision

- Each provider has a backward-compatible `command_completion` opt-in, default
  false. The settings control explains which context is sent. Only the selected,
  enabled, opted-in provider receives predictions; no environment-key fallback.
- A simple command word followed by a space is the initial analysis boundary.
  Wait 600 ms after input stabilizes. Partial words can still use local history.
  Each completed-word prefix is attempted once per 60 seconds, with a bounded
  per-pane record of 16 attempts and one cached candidate.
- All panes share a single in-flight HTTP admission. A completed request has a
  three-second cooldown; a failure backs off for 15 seconds. HTTP has an eight-
  second total timeout. Dropping the UI task marks cancellation but does not
  release admission while the blocking HTTP worker is still running.
- History lookup and provider/credential I/O stay on workers. Send at most eight
  related history commands from the same environment, each at most 512 bytes,
  plus bounded directory/environment and one prior assistant command. No terminal
  output or file scan is added. The existing redactor filters recognized secrets;
  this is best-effort redaction, not a guarantee against arbitrary secrets.
- Suggestions must extend the exact input prefix and contain a bounded single
  line without controls. Known dangerous suggestions are excluded. A model may
  decline; local suggestions remain available. Candidate validation happens on
  admission, not as a repeated regex scan of cached text during painting.
- UI ownership includes pane/session, directory, environment, provider generation,
  input epoch and native editor revision. Results cannot cross these boundaries.
  Alternate screens, unproven prompts, IME composition and midline edits do not
  start predictions. SSH stage transitions explicitly reset prediction state.
- Tab and Right accept a displayed AI suffix through the normal edit operation;
  neither sends Enter. Existing popup selection remains available, with AI rows
  marked in their labels. Escape cancels/dismisses the current prediction.
- The user may opt in to automatic model requests without enabling the separate
  failure monitor. A compatible prior assistant recommendation is optional context,
  not a requirement to open the assistant panel before command completion works.

## Rejected alternatives

- Request on every keystroke: avoidable cost and many obsolete responses.
- Await AI before showing local candidates: network latency would block useful
  history/path completion and make provider failures affect ordinary editing.
- Reconstruct commands only from keypresses: native shell editing and remote
  echo can invalidate that reconstruction; reuse verified prompt snapshots.
- Release the network slot when a pane stops waiting: synchronous HTTP could keep
  running and repeated edits would create overlapping requests despite the limit.
- Submit on Tab: the user explicitly chose Tab to fill and Enter to execute.

## Consequences

Quoted executable paths and commands without a completed word do not trigger
prediction in this version. Predictions require a recognized shell prompt and
enabled command suggestions. Unknown remote dialects are described as unknown.
Provider configuration changes saved by this process invalidate pending/cached
results; external edits are read by subsequent workers, not watched continuously.
The model may still produce an incorrect command, so acceptance remains explicit.

## Validation

Regression tests cover word admission, bounded/filtered context, prefix/control
validation, single-request admission/backoff, debounce, scoped history, cached
suffixes, Escape, stale input/directory/session/editor results, and the visible
settings switch. A rendered terminal fixture exercises prompt capture, AI ghost
text and real window Tab/Enter dispatch. Provider protocol regression uses local
HTTP fixtures; no real account credentials or model service are used in tests.
Windows compilation and targeted behavior tests are required for this change;
macOS/Linux native behavior and real-provider prediction quality need separate
acceptance. Test results are reported with the implementation, not assumed here.

## Supersedes

None. Extends the existing completion request and editor ownership boundaries.

## Revisit when

The provider interface supports cancellable asynchronous HTTP, or verified native
editor snapshots allow useful prediction beyond end-of-line prefix completion.
