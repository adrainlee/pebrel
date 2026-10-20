# Terminal assistant visibility and conversation ownership

## Status

Implemented locally; pending maintainer review.

## Context

The first adviser used a terminal-local overlay and an icon-only entry. A user
could not find either the assistant or its provider settings in the Windows test
build. The requested workflow is a collapsible right sidebar for conversation,
command explanations and selected-log analysis.

## Evidence

The settings navigation explicitly hid the providers route. Terminal-local
control tests bypassed workspace navigation and therefore could not prove that
users could reach configuration. The overlay also occupied the terminal's
content area and discarded its draft when closed.

## Decision

The workspace owns the assistant column's visibility and active-pane binding.
The terminal retains conversation, drafts, monitoring and request lifetime.
An observing surface holds a weak terminal reference, so switching or closing
panes cannot keep a PTY alive through the sidebar. The column reserves layout
space instead of covering the grid. File/document panels and the assistant use
the same right-hand location, with explicit toolbar selection between them.

Both tab layouts expose a text-labelled AI assistant button. The provider route
is visible in Settings, and the assistant opens that exact route after clearing
an unrelated settings search. Collapsing cancels the pending manual request but
retains completed exchanges and the draft. Switching panes binds the sidebar to
the new pane's own conversation. Opening settings preserves the dock selection.

Chat requests reuse the configured-provider transport and command validation.
Only successful manual exchanges enter history, bounded to six pairs and 4096
bytes per field, with the existing redaction applied at storage and serialization.
Explicit New chat clears manual history and retry snapshots. Automatic failure
analysis remains separate, opt-in, and dependent on command completion metadata.

## Rejected alternatives

- Keeping the overlay and adding another icon: it does not provide the requested
  side-by-side workflow or repair the hidden configuration route.
- Moving conversation state into the workspace: pane switches could mix contexts,
  and a second task owner would complicate pane shutdown and stale-result checks.
- Treating a component test or successful build as visual acceptance: neither
  verifies the packaged entry point, settings reachability or desktop rendering.

## Consequences

No new dependency or persistent format is introduced. Conversation history is
in memory and disappears when its terminal closes. Canceling a UI request still
cannot retract context already sent or interrupt the blocking HTTP transport.
Real provider/model compatibility remains separate from loopback HTTP testing.

## Validation

The workspace interaction test clicks the visible entry, checks right-column
geometry, switches terminal ownership, opens the provider editor and collapses
the dock in both tab layouts. Terminal tests exercise chat follow-up, retained
drafts, history reset, cancellation and a fake PTY channel. Native screenshots
and loopback-provider acceptance are recorded separately under ignored tmp/.

## Supersedes

The panel placement and close/discard behavior of
[the adviser lifecycle note](../terminal/view/2026-10-18-terminal-adviser-lifecycle.md).
Its transport, monitoring, insertion and stale-result invariants still apply.

## Revisit when

Conversation persistence, streaming, a separately selected assistant model or
cross-pane conversations become explicit product requirements.
