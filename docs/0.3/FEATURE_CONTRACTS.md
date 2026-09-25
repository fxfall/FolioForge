# FolioForge 0.3 GUI Feature Contracts

Status: active implementation contract

Date: 2026-09-25

This document records the Core-owned state, actions, and results used by the
current desktop workflows.

## Conversion

| Contract field | Definition |
| --- | --- |
| State | Core `FormatRegistry` input/output descriptors; selected Core `Target`; `ConversionOptions`; Core preflight approvals; `BatchInput`; output directory; Core progress/report; transient selected queue rows. |
| Actions | Add files/folder using Core-advertised extensions; select/remove queue items; select target; set conversion options; select output folder; preflight; convert selected/all approved; cancel the active `CancellationToken`; reveal the returned output path. |
| Events/results | `PreflightReport`; `BatchProgressEvent` stage/current/total/fraction; `BatchReport` and per-item result; cancellation result. UI localizes stage/severity and preserves diagnostic codes/details. |
| Capabilities | `FormatRegistry`, Core preflight, `folio-batch`, Core target/options types. The desktop interface does not define a separate support list or compatibility rule. |
| Localized presentation | Stable GUI keys; paths, extensions, format IDs, diagnostic codes, and source metadata remain data, not translated labels. |

Conversion requests use the public Core fields for target, deterministic
output, compression, degradation mode/options, text-import options, batch
mode, collision policy, tree preservation, and optional per-book edit plan.
Unspecified scheduler settings retain Core defaults. Text-import choices map
to `folio_text::TextImportOptions`; unavailable selections fall back to the
Core default or are rejected before conversion.

## Reader

| Contract field | Definition |
| --- | --- |
| State | Core Reader session, current `ReaderLocation`, direction-aware index, document index, Reader page/spread DTO, image resources and diagnostics. |
| Actions | Open/close; first/previous/next/last or select a Comic page by Core document index; set direction, spread mode, viewport/content mode, zoom, or pan. |
| Events/results | Core Reader location, render placements, resource identity/bytes, geometry, page count, and typed diagnostics/errors. |
| Capabilities | `folio-reader` operations exposed through Core and the public FFI contract. |
| Localized presentation | GUI labels map typed modes and diagnostic identifiers; resource IDs and geometry are never inferred or rewritten. |

Pointer gestures maintain temporary drag origin/delta and send viewport
changes to Core. The interface does not calculate crop, page order, geometry,
or spread membership. Encoded resource bytes are decoded only for display.

## Comic workspace

| Contract field | Definition |
| --- | --- |
| State | Core Comic session and `ComicPageDto` list; stable Core page IDs; Core-provided target descriptors; existing Comic-backed Reader projection; output result. |
| Actions | Open CBZ/ZIP/image directory; select a Core page; navigate through Reader; change Reader display/direction/spread/zoom/pan; select a Core-supported output; convert and cancel. |
| Events/results | Core page order/identity, thumbnail/preview, Reader placement/resource, progress, diagnostics, conversion report, and cancellation. |
| Capabilities | `folio-comic`, `folio-reader`, `comic_conversion_options`, and Core Comic conversion API. |
| Localized presentation | Page counts, controls, progress, empty states, and dialog labels are GUI strings; source names and metadata remain unchanged. |

The current Core exposes no Comic crop/split/rotation edit request. The desktop
interface must not invent such an editor or imply those transformations are
supported. If a Comic edit capability is added later, it must first be exposed
as a Core contract before becoming a desktop action.

## Supported behavior

The desktop exposes only Core-supported formats, conversion targets, Reader
operations, and Comic outputs. Comic input is limited to supported image
folders and ZIP/CBZ sources; CBZ is the current comic output. Unsupported
sources or transformations remain unavailable and are reported explicitly.
