# FolioForge 0.3 GUI Feature Contracts

Status: active implementation contract

Date: 2026-09-25

SwiftUI and Slint are sibling clients. This document records the semantic
inputs, actions, Core authority, and returned state needed to port the current
desktop workflows without re-implementing domain behavior.

## Conversion

| Contract field | Definition |
| --- | --- |
| State | Core `FormatRegistry` input/output descriptors; selected Core `Target`; `ConversionOptions`; Core preflight approvals; `BatchInput`; output directory; Core progress/report; transient selected queue rows. |
| Actions | Add files/folder using Core-advertised extensions; select/remove queue items; select target; set conversion options; select output folder; preflight; convert selected/all approved; cancel the active `CancellationToken`; reveal the returned output path. |
| Events/results | `PreflightReport`; `BatchProgressEvent` stage/current/total/fraction; `BatchReport` and per-item result; cancellation result. UI localizes stage/severity and preserves diagnostic codes/details. |
| Capabilities | `FormatRegistry`, Core preflight, `folio-batch`, Core target/options types. The frontend does not define its own support list or compatibility rule. |
| Localized presentation | Stable GUI keys; paths, extensions, format IDs, diagnostic codes, and source metadata remain data, not translated labels. |

Both clients map to the same request fields: target, deterministic flag,
compression, degradation mode/options, text-import options, batch mode,
collision policy `Rename`, tree preservation, and edit plan. SwiftUI omits
the Rust batch scheduler fields; their deserialization defaults are the same
values the Slint adapter supplies explicitly (`max_concurrent_jobs = 0`,
`per_job_parallelism = 1`, no byte budget, `fail_fast = false`). Each selected
input carries its output root and optional per-book edit plan. Slint's TXT
controls map one-to-one to `folio_text::TextImportOptions`; unrecognized UI
indices fall back to the Core default and are checked by the private static
mapping audit.

## Reader

| Contract field | Definition |
| --- | --- |
| State | Core Reader session, current `ReaderLocation`, direction-aware index, document index, Reader page/spread DTO, image resources and diagnostics. |
| Actions | Open/close; first/previous/next/last or select a Comic page by Core document index; set direction, spread mode, viewport/content mode, zoom, or pan. |
| Events/results | Core Reader location, render placements, resource identity/bytes, geometry, page count, and typed diagnostics/errors. |
| Capabilities | `folio-reader` through Core; direct Rust calls in Slint and the existing Reader FFI in SwiftUI. |
| Localized presentation | GUI labels map typed modes and diagnostic identifiers; resource IDs and geometry are never inferred or rewritten. |

The Slint pointer gesture only maintains temporary drag origin/delta and sends
viewport changes to Core. It does not calculate crop, page order, geometry, or
spread membership. The UI decodes the returned resource bytes only for widget
transport.

## Comic workspace

| Contract field | Definition |
| --- | --- |
| State | Core Comic session and `ComicPageDto` list; stable Core page IDs; Core-provided target descriptors; existing Comic-backed Reader projection; output result. |
| Actions | Open CBZ/ZIP/image directory; select a Core page; navigate through Reader; change Reader display/direction/spread/zoom/pan; select a Core-supported output; convert and cancel. |
| Events/results | Core page order/identity, thumbnail/preview, Reader placement/resource, progress, diagnostics, conversion report, and cancellation. |
| Capabilities | `folio-comic`, `folio-reader`, `comic_conversion_options`, and Core Comic conversion API. |
| Localized presentation | Page counts, controls, progress, empty states, and dialog labels are GUI strings; source names and metadata remain unchanged. |

The 0.2 Core currently exposes no Comic crop/split/rotation edit request. Neither
frontend may invent such an editor or imply those transformations are
supported. If a Comic edit capability is added later, it must first be exposed
as a Core contract, then consumed by both clients.

## Portability evidence

The ignored maintainer regression `tests/comic-ingestion/tests/gui_integration.rs`
drives a natural-ordered image directory through Comic Core, Core thumbnails
and preview, Reader page selection/resource-byte identity, CBZ export,
cancellation, and re-import. It is evidence for the Core feature contract,
not a substitute for hosted Windows/Linux builds or interactive visual QA.
The ignored static GUI parity audit checks every text-import selection-to-Core
enum mapping and the serialized defaults. It does not launch or interact with
either GUI. `docs/0.3/GUI_ARCHITECTURE.md` remains the ownership and layering
contract.
