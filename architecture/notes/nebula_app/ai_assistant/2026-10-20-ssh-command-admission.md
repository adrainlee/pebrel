# Command composer in an unintegrated SSH shell

## Status

Implemented; native SSH acceptance remains separate from the terminal fixtures.

## Context

A user enters `ssh` in PowerShell and then opens the AI assistant in the remote
shell. The SSH client remains a running local command. The remote shell may not
emit OSC 133 or editor reports, so requiring an active semantic prompt rejects
both `/ai` and insertion into an otherwise idle remote prompt.

## Evidence

The previous composer tests supplied OSC 133 markers even for guest environments.
They did not represent a manually entered SSH login without remote integration.
The existing shared grid reader already recognizes ordinary remote prompts, and
the shell-readiness check separately recognizes empty prompts without a typed tail.

## Decision

Reuse those shared grid rules. Expose the recognized empty prompt from the existing
readiness implementation without relaxing completion's nonempty-mirror contract.
Only a tracked interactive SSH login may bypass the local running-command veto;
ordinary running commands, authentication questions and alternate screens do not.
No remote helper installation or local process-tree inference is required.

Bind command authorization to the exact observed prompt as well as the existing
terminal, input, directory, environment and provider ownership. Capture the command
start identity so a new foreground command revokes the preview. After confirmation,
partial SSH echo waits within the existing deadline; only the full command echoed
at the same prompt admits Enter. A returned PowerShell prompt cannot receive the
delayed Enter even when the provisional SSH environment has not yet reconciled.

## Rejected alternatives

- Treating every live SSH client as busy: prevents input at its remote shell.
- Treating every SSH pane as idle: can send commands to authentication or TUI input.
- Submitting immediately after paste: ignores remote latency and target changes.
- Installing shell integration automatically: changes the server unnecessarily.

## Consequences

Recognition without shell integration remains heuristic and supports the existing
prompt forms. Unknown prompts or unverified echo remain unavailable or time out.
Ordinary advice fills the terminal without executing; `/ai` retains explicit
confirmation. SSH diagnostic reads still require a verified remote query channel;
typed SSH never falls back to local file reads.

## Validation

The GPUI command fixtures enter an SSH login, retain the running client, omit all
remote OSC markers, dispatch `/ai` and click real assistant controls. They cover
partial echo, changed prompts, password questions, running work and alternate
screens. These fixtures do not establish real-server compatibility.

## Supersedes

The semantic-prompt admission assumption in
[the command composer decision](2026-10-20-command-composer.md).

## Revisit when

A remote shell provides acknowledged editor operations or additional supported
prompt forms require stronger recognition.
