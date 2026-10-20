# Redact credential semantics before truncating provider context

## Status
Implemented; pending maintainer review.

## Context
A forty-character token heuristic passed short passwords, authorization values,
cloud access identifiers and URL userinfo into provider requests unchanged.

## Evidence
The review fixture reproduced all four classes. Commands, paths and conversation
history are provider context as well as output; legacy request construction must
not rely on callers having redacted every field.

## Decision
One shared redactor covers credential field names, authorization schemes, URL
userinfo, private-key blocks, known credential shapes and unknown long tokens.
Apply it to all request fields, including the legacy failure path. Bounded-tail
redaction includes up to 4096 bytes of preceding context before trimming. Omit an
incomplete leading line when its credential label lies outside that scan.

## Rejected alternatives
Increasing a token-length threshold still exposes ordinary passwords. Scanning
unbounded selections before trimming would move arbitrary work onto context capture.
Provider system prompts cannot enforce local privacy boundaries.

## Consequences
This is best-effort redaction, not a guarantee that arbitrary terminal content is
safe to disclose. Ambiguous credential-like text can be over-redacted, and very
long partial lines may be omitted. Automatic advice remains opt-in.

## Validation
Tests exercise short and long credentials, benign context, all provider payload
fields and a truncation boundary that removes the original credential label.

## Supersedes
None.

## Revisit when
Explicit context-selection or provider policies replace heuristic disclosure controls.
