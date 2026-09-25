# FolioForge Comic Core Architecture

This contract is for FolioForge 0.2. It extends the 0.1 Core additively and
keeps comic source interpretation, generic book meaning and output rendering
in separate layers.

## Flow and ownership

```text
folder / archive / PDF / eligible fixed-layout EPUB
                     |
                     v
          folio-comic::source
                     |
                     v
       ComicNativeBook (immutable)
                     |
          +----------+----------+
          |                     |
          v                     v
  ComicAnalysis           ComicEditPlan
 suggestions only          user authority
          +----------+----------+
                     v
          ComicComposition
          target-independent
                     |
                     v
       generic FixedLayout IR
             /               \
            v                 v
      folio-reader       optional BookEditPlan
            |                   |
   fixed-page model       folio-compat
            |                   |
        FFI / GUI          ComicRenderPlan
                                |
                       Rust image pipeline
                                |
                         exporters
```

`folio-comic` owns source decoding, source IDs, analysis, edits, composition,
comic layout semantics, target profiles, target render plans and output image
execution. Generic page display, viewport geometry, resource resolution,
reading order, navigation and display render models belong to `folio-reader`.
`folio-model` owns only reusable book semantics and resource loading.
`folio-edit` owns generic book edits. `folio-compat` owns the single target
capability matrix. `folio-core` integrates public operations; CLI/FFI/SwiftUI
consume Core/Reader DTOs and never interpret Comic semantics. The 0.2.1
migration now uses Reader render models for the supported Comic page subset;
the explicit Core-preview fallback remains for typed unsupported shapes or
transport limits.

## Dependency gates

Required boundaries:

- `folio-model` must not depend on `folio-comic`.
- `folio-comic` may use generic model/input/execution contracts, but must not
  depend on SwiftUI, Library, network providers, or format writer internals.
- `folio-reader` depends on the generic model/resource contract only; it must
  not depend on Comic internals, format importers/exporters, Core, FFI, Service,
  Library, SQLite, or GUI frameworks. `folio-comic` and `folio-model` do not
  depend on Reader.
- `folio-core` may call `folio-comic`; avoid a reverse dependency that creates
  a cycle. Shared cancellation/progress/execution contracts must be placed at
  a lower reusable boundary if the audit proves that necessary.
- Existing exporters receive generic IR plus target projection, never comic
  editor types.
- No output-specific algorithm lives in a GUI or the source decoder.

Do not split `folio-comic` into more crates until a real reusable boundary is
demonstrated. Do not add a second independent page scheduler; reuse or extract
the existing bounded execution contract after C1 establishes the narrowest
shared design.

## Native source, analysis and composition

`ComicNativeBook` records metadata, stable source page identities, source
locations, original dimensions/formats, explicit source order/structure and
source properties. Encoded resources are referenced lazily; decoded full-page
buffers are not stored in the native model. It contains no target profile,
output format, generated crop or inferred spread fact.

The C2 identity scheme is versioned BLAKE3 domain separation: an importer
supplies a stable, non-temporary container key; `SourceBookId` hashes that key
and the source kind, while `SourcePageId` hashes the book identity, typed
source location and an optional stable discriminator. The key itself is not
retained. Page identity is independent of vector index and source ordering, so
adding or reordering entries does not change the IDs of existing locations.
It intentionally does not hash full image bytes. Importers must provide a
stable key for the same source across reopening and must not use scratch paths.

Natural order is one Core comparator: NFC-normalized lowercase Unicode text,
ASCII digit runs compared by magnitude without integer conversion, then a
stable tie-break that places longer zero-padded equal-number runs first (the
pinned KCC 11.3.2 probe orders `001, 1, 002, 2, 010, 10`). Remaining ties use
case/Unicode order. This applies to paths, not semantic reading direction.
Sources with a parser-proven sequence (such as an EPUB spine) preserve that
explicit sequence after validating it is a permutation of all page IDs.

Stored member locators preserve their original Unicode spelling and accept
POSIX `/` separators only; `.`/`..`, absolute paths, control characters and
backslashes are rejected. A folder importer converts platform `Path`
components into a safe POSIX locator before constructing the model. The
comparator normalizes for ordering only; it never rewrites the source locator
used to reopen the resource.

`ComicAnalysis` is a separate immutable result. Every automatic output is
tagged as detected/suggested and certain/uncertain with evidence and
diagnostics. It never rewrites source truth. User overrides always win.

`ComicComposition` is built from source + analysis + edit deltas. It contains
effective order, pages, crops, rotation, spread/sides, panels, Webtoon slices,
chapters and cover, but remains target-independent. Rebuilding with the same
inputs and plan must be deterministic.

## FixedLayout IR boundary

Audit `folio-model` first. Reuse `LayoutMode::Fixed`, resources, navigation,
styles and presentation intent. Add generic types only for concepts useful to
non-comic fixed-layout books, such as reading direction, fixed page, viewport,
placement, page side and spread hint. Keep source crop suggestions, manga
detection, Webtoon split anchors, device profiles and comic edit commands in
`folio-comic`.

Projection is a one-way conversion from `ComicComposition` into generic IR.
After projection, the compatibility planner and exporters operate on generic
book meaning. Fixed-layout output is declared only when that IR and the target
writer can represent the page geometry and resource relationships.

### Initial source-only projection slice

The current implementation exposes `ImportedComic::to_semantic_ir()` for the
bounded source formats already accepted by the Comic importer. Raster sources
produce one generic fixed-layout document and image resource per validated
source page, in authoritative source order. Image bytes remain lazy and are
loaded from the originating source; the projection does not decode, transform,
crop, split, or infer reading direction. Fixed Layout EPUB retains and returns
the Semantic IR produced by the EPUB reader instead of reconstructing it from
page images.

This is a source-identity bridge, not completion of C6/C7. The current generic
model/projection does not recover missing viewport geometry, page side/spread,
manga direction, or source alt text. It emits explicit diagnostics for these
unknowns; no output-fidelity claim follows from the projection. PDF vector
pages require later rasterization, and unsupported raster formats fail closed.

The 0.2.1 Reader's initial R3 display path accepts the projection's explicit
single-image-per-document shape. When there is no source-declared viewport,
Reader may use the image's intrinsic dimensions as a transient display
coordinate space and reports that fallback in its render model. This improves
on-screen display only; it does not fill the generic IR geometry gap, remove
the Comic projection diagnostics, or establish target-output fidelity. The
Comic GUI now uses the Reader FFI/UI path for this supported shape. The typed
Core-preview fallback remains for unsupported page trees or transport limits;
this does not establish target-output fidelity.

The EPUB reader accepts only the exact DAISY 2005 NCX public/system identifier
pair when it appears as an external NCX doctype without an internal subset. It
removes that declaration before strict XML parsing; it never fetches or expands
an external DTD. Other external identifiers and all internal subsets remain
subject to strict parser rejection.

## Render and memory contract

`ComicRenderPlan` is inspectable, deterministic and target-dependent. It
combines generic fixed-layout IR, `TargetCapabilityProfile`, a physical
`ComicDeviceProfile` and `ComicOutputOptions`. It describes source crop,
manual rotation, automatic crop, panels/slices, resize, tone/gamma/contrast,
color/quantization, border and encoder in one explicit order.

The execution order is fixed by the render plan, not by helper call order:

1. Decode an active page once.
2. Apply source orientation correction and normalized manual crop.
3. Apply automatic crop only if selected and compatible with the manual crop.
4. Compose spread/panels/Webtoon slices.
5. Apply target geometry, color/tone and borders.
6. Encode/spool once, then release the decoded buffer.

Peak decoded memory is bounded by active workers × largest active decoded
page, not the total page count. Workers are bounded; results are collected in
semantic order. Cancellation and progress use Folio Core execution contracts.

## Security and determinism

Treat folders, archives, PDFs, image dimensions, metadata and serialized edit
plans as untrusted. Reject traversal, unsafe member names, duplicate identity
collisions, symlink escapes, nested archive abuse, excessive expansion, huge
dimensions, malformed pixels and invalid normalized coordinates. Limits are
explicit and reported. Never depend on filesystem/archive enumeration order.

The same source manifest, edits, processing version, options and profile must
produce the same composition, reports and deterministic output. Preview cache
is derived only and cannot store user edits or semantic truth.
