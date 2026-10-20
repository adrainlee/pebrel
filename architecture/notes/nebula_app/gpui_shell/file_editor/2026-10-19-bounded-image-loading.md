# Decode document images before admitting them to the cache

## Status
Implemented; pending maintainer review.

## Context
A retained-image cache budget cannot bound allocations made before cache insertion.
The old GIF rewrite path decoded and encoded PNG even when its disk cache existed.

## Evidence
A 43-byte GIF with a 4097-square logical canvas expanded to 67,141,636 RGBA bytes.
`images.rs` checked its temporary PNG cache only after conversion. GPUI's default
image loader also collected all animation frames for direct local image paths.

## Decision
Use one raster budget implementation for terminal, clipboard and document adapters.
Check encoded bytes, dimensions and RGBA cost before decoding, apply decoder limits,
and decode only the first frame. Documents retain their original image URLs and
source ranges; remove the eager temporary-PNG rewrite entirely. The document-owned
cache still owns completed pixels and failed loads.
Admit at most two document image jobs across all documents. HTTP image fetch and
conversion has a separate two-job bound; cancellation retains admission until
blocking work actually returns. Separate stages avoid nested-permit deadlock.

SVG remains on GPUI's renderer, with its existing 8192-dimension cap and a 1 MiB
encoded-source limit. Its raster ceiling differs from the 64 MiB raster-image
budget; parsing and embedded SVG resources are not claimed to be sandboxed here.

## Rejected alternatives
Checking bytes only after decode does not protect transient memory. A new metadata
cache would preserve avoidable conversion and filesystem lifetime. Reusing GPUI's
retain-all asset cache would defeat document suspension and eviction.

## Consequences
Document animations remain still images, including paths without a GIF extension.
Editing/reparsing no longer reads and transcodes every referenced local image.
Oversized or nonregular raster sources fail; small raster and SVG previews retain
their existing presentation. Limits are policy bounds, not total RSS guarantees.

## Validation
Tests cover oversized logical GIF canvases, encoded file limits, first-frame
selection, tiny per-document budgets and original Markdown source offsets.
Existing image-cache suspension and eviction tests remain required.

## Supersedes
None.

## Revisit when
GPUI exposes decode admission and configurable SVG budgets, allowing this adapter
to delegate without giving up the document's resource limits.
