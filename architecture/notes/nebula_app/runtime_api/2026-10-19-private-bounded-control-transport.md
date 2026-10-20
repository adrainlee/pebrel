# Private discovery and bounded control transport

## Status
Implemented; pending maintainer review.

## Context
The loopback discovery token authorizes terminal reads and command execution.
Default Unix creation modes could expose it through traversable configuration
paths. Subscription writes disabled their timeout and could retain all client slots.

## Evidence
`server.rs` used the ordinary atomic writer; its temporary file inherited umask.
`transport.rs` removed write timeouts before streaming and waited indefinitely
for snapshots after a peer disconnected.

## Decision
Keep the discovery path and wire envelope compatible. Create credential files
with mode 0600 before writing, inspect the opened inode, reject symlinks/hard links
and foreign ownership, and repair owner-created legacy records before reading.
Use OS randomness for instance tokens. Do not chmod an arbitrary config override
or its ancestors: a private file is the credential boundary.
Apply total read/write deadlines, buffer serialization, retain subscription write
limits, detect idle peer FIN, and unregister the subscription on every exit.
A FIN ends the subscription; subscribers must retain both socket halves.

## Rejected alternatives
Post-write chmod leaves a disclosure window. Changing every state file's mode or
moving discovery would broaden compatibility impact. A bounded snapshot channel
alone cannot interrupt a blocked socket write. Unbounded writes do not become safe
merely because the listener uses loopback.

## Consequences
Unix discovery can fail closed on unsafe paths. Old running instances rotate
previously exposed tokens only on restart. Slow clients reconnect after timeout.
The active-client guard also releases admission when thread creation fails.

## Validation
Regression tests cover credential replacement, legacy modes, link rejection,
oversized records, total deadlines, and idle subscription cleanup. Unix-specific
permission tests require Unix CI; Windows compilation cannot validate those modes.

## Supersedes
None.

## Revisit when
The control transport moves to an authenticated native IPC endpoint.
