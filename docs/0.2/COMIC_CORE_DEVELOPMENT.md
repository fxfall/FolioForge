# FolioForge 0.2 Comic/Manga Core Development Contract

| Field | Value |
| --- | --- |
| Status | In development |
| Target product version | `0.2.0` |
| Baseline | Frozen FolioForge `0.1.0` |
| Implementation language | Rust |

This document and the linked contracts under `docs/0.2/` define the 0.2 comic
work. The 0.1 contracts remain authoritative for the frozen 0.1 behavior. A
0.2 change must not silently rewrite the 0.1 release claims; code/document
mismatches are recorded as Architecture Drift before implementation changes.

## Scope

FolioForge 0.2 adds conversion-focused comic and manga processing through one
comic Core domain, then projects the result into the existing generic Folio
IR and target exporters. Reflowable ebook conversion remains available without
the comic layer, Library, GUI, network access or database state.

The original C0–C20 Comic Core track does not implement Library product
features, persistent Reader state, bookmarks, highlights, cloud sync or a
separate target-specific comic converter. Stateless display/navigation is
added separately by the 0.2.1 Reader Runtime contract; it does not expand the
Comic conversion scope. See [READER_RUNTIME.md](READER_RUNTIME.md).

KCC is a feature, behavior, usability and controlled A/B reference. It is not
a source-level implementation template. FolioForge must not copy its Python
module structure, execution flow, GUI, temporary-file design, option names or
external-tool assumptions. See [KCC_REFERENCE.md](KCC_REFERENCE.md) and
[COMIC_AB_PROTOCOL.md](COMIC_AB_PROTOCOL.md).

## Required pipeline

```text
Comic source
  -> bounded source decoder
  -> immutable ComicNativeBook
  -> read-only ComicAnalysis
  -> ComicEditPlan + semantic command
  -> target-independent ComicComposition
  -> generic FixedLayout Folio IR
  -> existing generic BookEditPlan when needed
  -> folio-compat target projection
  -> ComicRenderPlan and one-pass Rust image processing
  -> existing exporter
  -> validation and atomic output commit
```

There are three permanent ownership boundaries:

- `ComicEditPlan`: source-relative changes requested by the user.
- FixedLayout IR: generic book semantics and presentation geometry.
- `ComicRenderPlan`: physical output decisions for a target/device.

They must not be collapsed into one editor or moved into the GUI.

## Core rules

- Create `crates/folio-comic/`; do not create pair-specific converters.
- Native source facts are immutable. Analysis only emits evidence, confidence,
  suggestions and diagnostics. Explicit user edits override suggestions.
- Comic source edits remain in `folio-comic`; generic title/author/language,
  identifiers and navigation-label edits use existing `BookEditPlan`.
- Device profiles describe physical screens; `folio-compat` remains the sole
  output-format capability authority.
- Exporters consume generic IR and target projection only. They must not
  inspect `ComicEditPlan` or `ComicNativeBook`.
- Preview and final conversion share edit, transform and render planning.
- Decode each active page once, apply an inspectable transform plan, encode
  once, and release decoded buffers. Parallel work is bounded and ordered.
- Directory/archive/PDF inputs are untrusted. Enforce path, size, entry,
  nesting, decoded-dimension, memory, cancellation and time limits.
- No production runtime dependency on `.folioforge-dev/` or `.codex/`.

## Development gates

Each gate is completed and recorded before dependent work begins. Major-change
classification follows the 0.2 plan; native source, edit/composition models,
FixedLayout IR, geometry algorithms, RenderPlan, device profiles and parallel
processing are major changes and require independent A/B evidence.

| Gate | Deliverable | Exit condition |
| --- | --- | --- |
| C0 | Documentation and boundary freeze | All six 0.2 contracts reviewed; ownership and acceptance rules are stable. |
| C1 | Existing Core audit | Reuse, generic extension, comic implementation, missing and reject decisions recorded for every audited layer; drift recorded. |
| C2 | Native model | Stable source identities, immutable source data and deterministic page ordering. |
| C3 | Ingestion | Folder, CBZ/ZIP, CBR/RAR, CB7/7Z, PDF and eligible FixedLayout EPUB have safe, equivalent native representations. |
| C4 | Analysis | Dimensions/orientation/color and initial crop/spread/panel suggestions; read-only and confidence-tagged. |
| C5 | Editing Core | Delta plan, semantic commands, validation, serialization, page/cover/crop/rotation/spread/direction/chapter/panel/Webtoon edits. |
| C6 | Composition | Native source + analysis + edits produce deterministic effective comic structure. |
| C7 | Generic FixedLayout IR | Only generic cross-format concepts are added; projection is source-blind after this boundary. |
| C8 | Comic preview planning | Source/edited/target render plans and thumbnails use production Comic planning/transforms; generic display is owned by the 0.2.1 `folio-reader` contract. |
| C9 | Render planning | IR + device + target + options yield inspectable deterministic ComicRenderPlan. |
| C10 | Image execution | Crop, rotate, resize, tone, gamma, contrast, color, borders and selected encoders; bounded memory and benchmark. |
| C11 | Manga/spreads | LTR/RTL, detect/override, split, rotate, split+rotate, side and alignment. |
| C12 | Webtoon/panels | Automatic/manual segmentation, two/four-panel regions and inter-panel crop. |
| C13 | Device profiles | One versioned Core registry for Kindle, Kobo, reMarkable and custom dimensions. |
| C14 | Basic outputs | Image folder, CBZ and validated EPUB 3 Fixed Layout. |
| C15 | Advanced outputs | KEPUB-compatible EPUB, PDF and KF8/AZW3; KFX fixed layout only after separate capability proof. |
| C16 | Advanced processing | Smart cover/crop/fill, target-size planning, fusion, volume splitting, quantization, E-Ink and screentone/rainbow mitigation. |
| C17 | CLI | `folio comic inspect|analyze|convert` and supported shared conversion routes use Core APIs. |
| C18 | Performance | Representative workload baseline and profile-led optimization. |
| C19 | GUI API freeze | Stable Core DTOs for inspection, edit commands, previews, profiles, output options and reports; FFI impact documented. |
| C20 | Final regression | 0.1 regression, comic corpus, controlled KCC A/B, platform builds, security/fuzz, architecture, performance and Redundancy Zero. |

No production implementation starts before C0 is complete. C1 findings are
recorded in [COMIC_FEATURE_MATRIX.md](COMIC_FEATURE_MATRIX.md). The current
candidate drift is `AD-0.2-001`: the 0.1 capability table calls EPUB Fixed
Layout Native, while the current EPUB writer does not serialize fixed-layout
package metadata or per-page viewport geometry. C7/C14 must resolve this
against controlled fixtures before claiming support.

## Final acceptance

The final checklist is the 0.2 definition of done in the supplied plan and is
summarized in [COMIC_FEATURE_MATRIX.md](COMIC_FEATURE_MATRIX.md). A feature is
not complete because a type or CLI flag exists: its behavior, safety,
determinism, diagnostics, tests, A/B evidence and output validation must pass.

Every implementation batch records compilation, targeted tests, 0.1
regression, architecture checks, applicable A/B results, cleanup and
Redundancy Zero. Build and scratch data use `FOLIOFORGE_VALIDATION_ROOT`.
Private corpora, A/B artifacts and chronological work notes stay in ignored
local directories and are never included in releases or Docker contexts.
