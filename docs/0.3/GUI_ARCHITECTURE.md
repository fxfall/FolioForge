# FolioForge 0.3 Desktop Feature Contract

- Product version: `0.3.1`
- Release contract: current

This document records the desktop workflows and their integration boundary.
All user-visible decisions about formats, compatibility, reading order,
diagnostics, and conversion remain owned by the Rust Core.

## Responsibilities

- Core owns format detection and support, conversion defaults, the Semantic
  IR, compatibility decisions, diagnostics, progress, cancellation, Reader
  semantics, Comic page identity/order, and output behavior.
- Desktop presentation owns transient view state, file/folder dialogs,
  accessibility, keyboard/pointer gestures, layout, localization, and display
  of Core-provided resources and render models.
- Every conversion, Reader, and Comic action uses the corresponding public
  Core or FFI contract. A view must not parse an ebook or maintain a second
  capability registry.
- Interface text may be localized; file extensions, format identifiers,
  diagnostic identifiers, error codes, source metadata, and resource identity
  remain unchanged.

## Conversion workflow

The application discovers inputs and selectable output targets through Core,
requests preflight before conversion, and presents the resulting compatibility
plan and diagnostics. Conversion uses the Core request, batch, progress,
cancellation, report, and atomic-output contracts. The interface does not
invent defaults or compatibility rules.

The request preserves Core settings for target, deterministic output,
compression, degradation mode/options, text-import options, batch behavior,
collision handling, and per-book edit plans. Invalid or unavailable choices
must resolve to the Core-provided default or be rejected before conversion.

## Reader workflow

Reader displays Core-owned sessions, locations, page/spread render models,
resource identity, geometry, diagnostics, and errors. Navigation, direction,
spread mode, viewport, fit mode, zoom, and pan are submitted through Reader
operations. The desktop layer only transports encoded image resources to its
display widgets; it does not infer page order, crop, geometry, or spread
membership.

## Comic workflow

The supported desktop comic workflow opens image folders and ZIP/CBZ sources,
shows Core-ordered pages and thumbnails, previews pages through Reader, and
exports supported CBZ output. Inputs outside the accepted Comic Core subset
must show a typed unsupported result; the interface must not infer missing
source semantics or advertise unfinished transformations.

## Localization and accessibility

English and Simplified Chinese are supported. Required messages need translated
values and matching placeholders. Long translations must remain readable;
only secondary metadata may be truncated. Controls retain accessible labels,
keyboard/pointer operation, and typed error handling.

## Supported desktop packages

The current release packages target macOS 27+ ARM64, Linux x86_64 and ARM64,
and Windows x86_64 and ARM64. Every package, app bundle, Core component, and
version response must use the release tag's same product version.

## User-visible guarantees

- Format, target, and compatibility choices reflect the current Core support
  contract.
- Diagnostics retain their stable codes and explain rejected or approximated
  input/output behavior.
- Long operations expose progress and cancellation where Core supports them.
- Reader navigation and Comic page order come from Core; unsupported inputs
  and transformations are not guessed or silently advertised.
- Windows desktop launch does not open a console window; command-line tools
  retain normal console behavior.
