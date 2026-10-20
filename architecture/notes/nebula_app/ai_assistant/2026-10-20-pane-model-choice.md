# Pane-owned assistant model selection

## Status

Implemented with the existing configured provider records.

## Context

Multiple configured providers can be usable simultaneously. Requiring a trip to
Settings to change the global active provider obscures which model the assistant
will use and couples one pane's choice to unrelated completion defaults.

## Evidence

Provider records already own one model, endpoint and credential reference each.
The assistant owns its requests, generation numbers and command confirmations.
Previously the worker loaded the global active provider when it began executing.

## Decision

Show provider name and model in the assistant's existing dropdown controls. Each
pane owns its selection and retains it across collapse and mode changes. Offer
enabled records with a model, HTTP endpoint and credential metadata where needed;
this is configuration readiness, not a connectivity guarantee. Refresh records
when opening or selecting models and when requesting analysis, outside rendering.

Capture the selected non-secret provider record in the analysis request. Resolve
credentials only in the existing background provider adapter. A removed, disabled
or incomplete explicit selection reports an unavailable provider, without sending
context to an alternative endpoint. Choosing a model performs no model request.

Switching cancels owned analysis/query/submission tasks, advances the generation,
clears cached outcomes and revokes command confirmation. Preserve conversation and
draft text. Do not mutate the global provider default or command-prediction opt-in.

## Rejected alternatives

- Persisting another model registry: existing provider records already own models.
- Changing the global active provider from a pane: changes unrelated panes/defaults.
- Looking up the active provider after dispatch: may disagree with the selected UI.
- Silently falling back when a selection becomes unavailable: changes the recipient.

## Consequences

Users choose among configured provider/model records; this does not discover every
model advertised by a remote service. The choice lasts for the pane's lifetime.
Requests already sent cannot be withdrawn, but their canceled results cannot
restore a command confirmation or submit Enter.

## Validation

GPUI fixtures use the visible picker and keyboard menu navigation, verify request
snapshots and global-default isolation, and reject stale results, delayed Enter
and unavailable selections. A local HTTP fixture verifies the actual selected
endpoint and model. Real-provider availability is outside those fixtures.

## Supersedes

None.

## Revisit when

The product adds provider model discovery or persists per-conversation models.
