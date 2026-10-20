# Bound terminal semantic-event delivery

## Status
Implemented; pending maintainer review.

## Context
Repaint coalescing did not bound BEL, title, command or OSC payload events.
A 200,000-byte BEL fixture retained 200,000 events while the UI consumer was idle.
Both local PTY and authenticated SSH output share this mailbox.

## Evidence
`event_mailbox.rs` previously coalesced only Wakeup. The UI processes batches of
128, so producer memory was independent of terminal scrollback limits.

## Decision
Share admission accounting among sender clones. Bound ordinary pending events to
4096 and owned payload capacity to 32 MiB. Coalesce pending repaint and bell events;
rate-limit bells to one per 100 ms. Normal semantic events preserve their order.
On overflow enqueue one failure and one Exit in reserved control capacity, reject
further events, and have the existing PTY-failure handler shut down the transport.
No producer waits for consumer capacity while holding the terminal grid lock.

## Rejected alternatives
Blocking bounded sends can deadlock against the UI's terminal lock. Dropping
command/Agent edges silently corrupts lifecycle state. Coalescing all titles is
unsafe because integrated titles can contain shell metadata, not only decoration.

## Consequences
A session that overwhelms semantic delivery ends explicitly instead of consuming
unbounded memory. This is an overload failure policy, not output-rate throttling.
The queue bounds payload capacity conservatively, including shared image buffers.

## Validation
Tests cover 200,000 bells, ordinary command/Agent ordering across 80 sessions,
count and byte overflow, reserved failure delivery, clone sharing and EOF.
The original deterministic fixture was reproduced on Windows; no universal
throughput guarantee is inferred from that fixture.

## Supersedes
None.

## Revisit when
The producer can yield safely outside the terminal lock or a protocol provides
lossless backpressure for semantic events.
