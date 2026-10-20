# Explicit command generation and reviewed submission

## Status

Implemented for the GPUI assistant, pending the validation recorded with the change.

## Context

The user requested an explicit natural-language command composer and execution
only after reviewing and confirming the generated command. The final entry is
the exact `/ai` command followed by Enter; a single slash must remain normal input.

## Evidence

- The existing assistant owns provider requests, redaction, input focus, errors,
  cancellation and generated command validation.
- Terminal prompt snapshots distinguish shell input from running applications.
- Existing terminal paste and keyboard encoders preserve bracketed paste and
  negotiated Windows input protocols. A literal CR is not valid for every mode.

## Decision

- Intercept Enter only when a verified idle shell input line is exactly `/ai`.
  Remove those three characters through the normal Backspace encoder and open
  the assistant's command composer without submitting the application command.
- Reuse the assistant with a distinct `generate_command` task. Typing a natural
  language description performs no network request. Enter or the Generate button
  requests one command and its explanation; Shift+Enter allows a longer description.
- Show the command, explanation, risk indication and captured execution target.
  Only the explicit Confirm and run button authorizes submission. Ordinary
  assistant explanations retain their existing insert-without-submitting action.
- Bind a preview to its terminal allocation, input epoch, editor revision,
  directory, environment and provider configuration generation. Input edits,
  changed context, closing, cancellation and SSH lifecycle changes revoke it.
- Confirmation captures the displayed generation and exact command, consumes the
  ready state once, and pastes only into the owning pane without broadcasting.
  Wait up to three seconds for exact command echo, then use the existing keyboard
  encoder and command tracking to submit Enter. Intervening input or context changes
  revoke the delayed Enter. Timeout leaves the text visible without submitting it.
- A weak reference identifies the terminal allocation without keeping a closed
  session alive. The assistant owns the submission task, so closing cancels it.

## Rejected alternatives

- A single `/` entry: the user explicitly rejected its ambiguity with shell paths.
- Running generated commands automatically: the user requires review and confirmation.
- Sending command text and CR as one raw string: bypasses paste/input encoding and
  allows a shell or terminal application to interpret submission differently.
- Sharing a confirmation across panes or directories: the reviewed target would
  no longer describe the terminal receiving the command.

## Consequences

The feature requires a recognized idle shell prompt. The exact `/ai` spelling is
reserved at that prompt; paths such as `/ai/bin` keep ordinary shell behavior.
Wrapped or unusual shell echo that cannot be verified may leave text unsubmitted;
the UI explicitly reports this instead of claiming execution. Submission feedback
means Enter was sent, not that the operation succeeded. Terminal output remains
the source of the command's result. Automatic command completion remains a separate
provider opt-in and is not required for this workflow.

## Validation

GPUI fixtures exercise actual terminal text input, Enter dispatch, assistant input,
visible confirmation controls, one-shot submission, and stale-context cancellation.
Protocol and bounded-context tests reuse the assistant's existing local HTTP fixtures.
No real model account or remote server is used by these tests; native interactive
acceptance and real-model command quality require separate testing.

## Supersedes

None. Adds an explicitly confirmed execution path to the existing assistant.

## Revisit when

Shell integration offers an acknowledged atomic replace-and-submit operation, or
the command composer gains a distinct editable command preview.
