# Comic Feature Matrix and Existing Core Audit

This is the C1 inventory for the 0.2 plan. Status meanings:

- **Reuse** — an existing Core contract can be used as-is.
- **Generic Extension** — add a format-neutral capability consumed beyond
  comic-specific code.
- **Comic-specific** — implement inside `folio-comic`.
- **Missing** — no usable current implementation; implement at the owning
  layer before claiming support.
- **Reject/defer** — reject or keep out of the public promise until evidence
  and an independent capability gate exist.

## C1 findings

| Area inspected | Current fact | C1 classification and consequence |
| --- | --- | --- |
| `folio-model` | Has `Book`, ordered documents/nodes/resources, navigation, styles, `LayoutMode::Fixed` and generic presentation intent. No fixed page, viewport, placement, reading-direction enum, page side or spread hint exists. | Reuse current generic pieces; **Generic Extension** for proven cross-format fixed-layout concepts at C7. Comic suggestions and edits stay out of this crate. |
| `folio-edit` | `BookEditPlan` edits metadata, cover, typography, fonts, styles, document order/titles and navigation labels. | **Reuse** for generic metadata and book semantics; **Comic-specific** `ComicEditPlan` for source page/crop/spread/panel/Webtoon edits. |
| `FormatRegistry` / `folio-core` | Registry owns ebook format adapters and file conversion. Core already owns cancellation, progress, validation, reports and atomic file output. | **Reuse** generic conversion contracts; **Generic Extension** for an explicit comic Core API/DTO and existing Core orchestration. Do not disguise comic inspection as a reflowable format adapter. |
| `folio-input` | Safe single-file, file-set and recursive directory sources; paths normalize and symlinks/traversal are rejected. Directory enumeration is lexically sorted, not natural page order. | **Reuse** safe path primitives; **Generic Extension** only where archive/member limits or streaming access are missing; **Comic-specific** authoritative natural ordering. |
| ZIP/archive | `zip` is used by EPUB/package code. `folio-archive` writes ZIP but always injects `FolioForge-report.json`; no comic archive decoder or report-free CBZ writer is exposed. No RAR/7z crate is in the workspace. | **Reuse** ZIP primitives where safe; **Missing** comic source extraction and report-free CBZ output; C3 must choose bounded Rust decoders for CBZ/CBR/CB7 or explicitly report dependency blockers. Never shell out to KCC tools. |
| Resource loading | Generic `ResourceLoader` supports bounded resource reads; existing importers often materialize format resources. No lazy comic page catalog or decoded-image lifecycle exists. | **Reuse** bounded generic resource interface where suitable; **Comic-specific** lazy page references and page-at-a-time processing. |
| Raster images | Workspace has no general image decode/transform/encode dependency or image pipeline. Existing format code can recognize some embedded image kinds but does not satisfy comic processing. | **Missing** decode/encode/transform, dimension caps and memory lifecycle. Select Rust dependencies at C3/C10 with format-specific tests. |
| EPUB Fixed Layout | Importer recognizes `rendition:layout=pre-paginated` and sets `LayoutMode::Fixed`. `Book` carries no page viewport geometry. The current EPUB exporter writes neither fixed-layout rendition metadata nor viewport properties. | **Architecture Drift AD-0.2-001**: `docs/0.1/FORMAT_SUPPORT.md` calls EPUB Fixed Layout Native, but the writer does not emit the required page geometry/package declaration. C7/C14 must add generic geometry and validate real Fixed Layout EPUB output before changing any support claim. |
| KF8 Fixed Layout | Current output target exists and carries HTML/CSS/resource structure; no generic per-page viewport/page-side model exists. | **Missing / evidence pending**: do not claim comic fixed-layout parity until C1 fixtures and C15 target tests establish a valid representation. |
| KFX Fixed Layout | 0.1 only promises an internal compatibility writer and DRM-free reflowable input. | **Reject/defer** until a separate native KFX fixed-layout capability proof exists. Reflowable KFX support is not evidence. |
| PDF input/output | No PDF crate, adapter, target or writer exists in the workspace. | **Missing**. C3/C15 require a separate Rust implementation and capability/security tests; no external process assumption. |
| Batch/execution | `folio-batch` has bounded job workers, progress, cancellation and a memory gate; its scheduler internals are private to that crate. Core has conversion cancellation/progress. | **Reuse** semantics; **Generic Extension** only if C10 proves a shared lower-level worker primitive is needed. No second independent unbounded page scheduler. |
| Output commit | Core has atomic file output. No atomic directory sink exists. ZIP report writer is not a CBZ contract. | **Generic Extension** for safe directory commit if required; **Comic-specific** archive/page naming and report policy. |
| Preview/API | Existing preview is for book/profile presentation; no comic page thumbnails, crop overlays or target raster preview API. FFI exposes stable JSON for existing services. | **Comic-specific** source/edited/target preview, then **Generic Extension** stable DTO transport at C19. |
| Capability matrix | `folio-compat` is the single target capability authority for EPUB/KF7/KF8/KFX; there are no comic device profiles or image encoder capabilities. | **Reuse** format capability authority; **Generic Extension** only for new generic fixed-layout semantics; **Comic-specific** physical device registry. |

## C0/C1 status

C0 documentation is frozen by the six contracts in `docs/0.2/`. C1 source
inspection establishes the rows above. `AD-0.2-001` is recorded before any
production modification; it does not retroactively alter the frozen 0.1
release claim. Its resolution is gated on FixedLayout fixtures and exporter
validation at C7/C14.

## C2 implementation status

The `folio-comic` Rust crate now contains the immutable native book/page
model, opaque versioned source IDs, safe typed source locations and validated
natural/explicit ordering. C2 is accepted: 11 local synthetic regression
tests, full workspace tests, formatting, Clippy, architecture, degradation
golden, CLI/FFI/service parity, and three fresh-process KCC 11.3.2 ordering
probes passed. The probe covered 16 synthetic pages with numeric, zero-padded,
Unicode, nested and CJK names. KCC and FolioForge order matched on every run;
the discovered zero-padding rule is documented in
[`COMIC_ARCHITECTURE.md`](COMIC_ARCHITECTURE.md). Local regression tests and
comparison artifacts remain only in the ignored repository-root `tests/`
bundle.
No image decoding or transform behavior is included in C2.

## C3 status

C3 is still in development and is **not accepted**. The current Rust adapter
builds native manifests and bounded lazy page access for image directories,
CBZ/ZIP, CB7/7Z, classic-xref PDF page catalogs, and a deliberately narrow
Fixed Layout EPUB subset (one supported raster image per spine item, with image
order taken from the EPUB spine). PDF pages are vector locations only; raster
bytes remain deferred to the later rendering gate.

The EPUB reader's NCX policy allows only the exact external DAISY 2005 NCX
public/system identifier pair with no internal subset. That declaration is
removed before strict XML parsing; no network fetch or DTD entity expansion is
performed. Other external DTDs and internal subsets are not allowlisted. C3
remains unaccepted because its other format/dependency gates, notably CBR/RAR,
are still unresolved.

The private Rust source-ingestion harness passes 13 controlled regressions for
natural folder/archive ordering, page-byte reads, path traversal rejection,
page-count limits, cancellation, fixed-layout spine mapping, PDF vector-page
mapping, and explicit refusal of unsupported RAR/7z/PDF structures. No Python
or PowerShell tooling is part of the public source tree.

| Input | Current C3 boundary |
| --- | --- |
| Directory | Symlinks and non-regular page files are rejected; supported raster extensions are JPEG, PNG, GIF and WebP. Page bytes are read one at a time and signature-checked. |
| ZIP/CBZ | UTF-8 safe member paths only; traversal, duplicate names, symlinks and encrypted members are rejected. Central directory is capped at 32 MiB. |
| 7z/CB7 | Unencoded, unencrypted headers only. Accepted methods are Copy, LZMA/LZMA2, BZIP2, PPMd and listed BCJ filters. Dictionary cap is 512 MiB; solid bytes decoded before a requested page are capped at 2 GiB. Encoded headers are rejected before parser invocation. |
| PDF | Strict parsing only for classic xref tables. Xref streams, hybrid xrefs, incremental `/Prev` chains and encrypted PDFs are deferred. Source cap 128 MiB, per-stream decompression cap 32 MiB and page cap 20,000. Imported pages remain vector references until a bounded rasterizer exists. |
| Fixed Layout EPUB | Requires pre-paginated layout and exactly one supported raster image in each spine item. Image media type must match its signature; spine order is authoritative. Other fixed-layout semantic content is rejected. A canonical external DAISY 2005 NCX DTD is accepted without fetching it; internal subsets and other DTD identifiers remain rejected. |
| CBR/RAR | Explicitly deferred. The reviewed MIT Rust crate documents format limitations that do not establish reliable RAR4/RAR5 coverage; the alternative reviewed reader carries GPL/unRAR licensing restrictions incompatible with automatic inclusion under the current project license. |

Default general limits are 8 GiB per file source, 16 GiB aggregate expanded
bytes, 512 MiB per archive member, 256 MiB per page, 100,000 entries, 20,000
pages and 128 path components. These are resource ceilings, not promises that
every accepted input variant will be supported. Dependency/license and
hostile-input review remains part of C3 acceptance.

## 0.2 delivery coverage

The required end-state is the complete 0.2 plan, not merely the C1 audit:

- Native model, stable source IDs, source order and immutable metadata.
- Folder, CBZ/ZIP, CBR/RAR, CB7/7Z, PDF and eligible FixedLayout EPUB input.
- Read-only dimensions/orientation/color, crop/spread/panel analysis with
  confidence and diagnostics.
- Serializable delta edit plan; validated page, cover, crop, rotation, spread,
  side, direction, chapter, panel, Webtoon and processing edits.
- Deterministic composition and generic FixedLayout IR projection.
- Source/edited/target previews and thumbnails through shared Core logic.
- RenderPlan, one-pass Rust processing, common device profiles, bounded memory
  and deterministic parallel page work.
- LTR/RTL, spread, panel, Webtoon, fusion, volume and advanced processing.
- Image folder, CBZ, EPUB Fixed Layout, KEPUB-compatible EPUB, PDF and KF8;
  KFX only after proof.
- Core APIs, CLI, GUI-facing stable DTO freeze, reports, validation,
  cancellation, progress, security/fuzz and performance baselines.
- Full 0.1 regression and platform builds for Linux x86_64/ARM64, Windows
  x86_64/ARM64 and macOS ARM64.

Each row changes from Missing to supported only after its gate's acceptance
evidence is recorded.

## Comic-to-Semantic-IR implementation slice

`ImportedComic::to_semantic_ir()` now bridges the supported raster-page source
subset to generic `Book`/Semantic IR using one document and one lazy image
resource per source page. Fixed Layout EPUB returns the already-imported EPUB
IR. This preserves page order and source bytes, but does not complete C6/C7:
viewport, side/spread, manga direction and alt-text semantics remain unknown
and are diagnosed. No rendering or output fidelity is claimed. The new API is
an additive 0.2 development API change; the frozen 0.1 API and support claims
are unchanged.
