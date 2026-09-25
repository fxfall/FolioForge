# Comic GUI/Core Integration Status

Date: 2026-09-24

Scope: the first FolioForge 0.2 Comic GUI path, from local source selection to
Core-owned preview and IR-backed output. The existing 0.1 book-conversion
workflow remains unchanged.

This record implements the GUI integration plan supplied for this task. The
plan's G0–G10 gates remain authoritative; this file records the local audit and
completion evidence, not a claim that every future GUI milestone is complete.

## G0 — Contract audit

| Capability | Baseline | Classification / decision |
| --- | --- | --- |
| Folder / ZIP / CBZ import | `folio-comic::ComicSourceImporter` supports safe folder and ZIP/CBZ ingestion with bounded lazy page reads. | READY in Comic Core; needs Core session and FFI exposure. |
| Session identity and lifecycle | No Comic session exists at the Core/FFI boundary. | NEEDS CORE API; use an opaque serialized session ID backed by Core-owned `Arc` state so closing cannot invalidate in-flight cloned requests. |
| Page list and stable identity | `ComicNativeBook` has validated source order and stable `SourcePageId`. | READY in Comic Core; needs stable JSON DTOs. UI indices are display-only. |
| Summary / metadata / warnings | Native metadata and source-to-IR diagnostics exist. | READY in Comic Core; needs a Core summary DTO and FFI transport. Unknown direction/geometry remain unknown. |
| Page dimensions | Model field exists but current raster import does not populate it. | NEEDS CORE API; probe one selected source page's bounded image header; do not scan/decode the full book on open. |
| Thumbnail and source preview | No image decoder, renderer, or preview cache exists. | NEEDS CORE API; add a bounded Rust decode/resize/PNG path in `folio-comic`, on-demand and cache-bounded. Animated image source bytes stay intact in output; preview is a representative still frame. |
| Comic target authority | Existing `folio-compat` exposes only ebook targets; no Comic target descriptor exists. | NEEDS CORE API; expose only implemented CBZ from Core/compatibility authority. Do not show EPUB/KF8/KFX or device choices without implementation/profile evidence. |
| Comic device profiles | No profiles or profile registry exist. | NOT YET IMPLEMENTED; omit the device picker and report this as a remaining plan gate, rather than duplicating profile definitions in Swift. |
| IR-backed output | Comic→Semantic IR exists; no report-free CBZ writer exists. Existing archive writer injects a FolioForge report and is not a CBZ contract. | NEEDS CORE API; implement a deterministic CBZ writer consuming only the projected generic IR and lazy resource loader. |
| Progress / cancellation | Generic Core conversion has both, but Comic has no operation. | READY as reusable contract; needs Comic operation events and cancellation checks at page boundaries. Image codec work is single-page and checks cancellation around decode. |

## Initial implementation boundary

Implement G0–G4's first usable path:

```text
Folder / ZIP / CBZ → Core Comic session → lazy page grid → source preview
                   → conversion request → generic Semantic IR → CBZ output
```

The first output is intentionally limited to CBZ because it can preserve
supported source image bytes without claiming fixed-layout geometry, device
rendering, or image transformation support. Device selection, semantic editing,
edited/target preview, other outputs, and large-corpus performance acceptance
remain deferred until their Core contracts exist.

## Change classification and boundary

- Classification: major 0.2 API/FFI/UI addition because it adds Comic sessions,
  page-preview operations and an output route outside the 0.1 API surface. It
  is additive to the 0.2 development line; frozen 0.1 behavior, formats,
  targets, API and release claims are unchanged.
- SwiftUI owns source selection, page selection, presentation, output-path
  choice, and request dispatch only. It does not parse archives, order pages,
  resize images, define targets/profiles, or write outputs.
- Source import and output both pass through generic Semantic IR; output
  consumes IR resources, not `ComicNativeBook` or GUI state.
- Local tests, generated fixtures, process records, and build scratch remain
  excluded from GitHub/release contexts per the repository rules.

## Implemented first GUI path

This implementation completes the first usable, Core-backed GUI slice—not all
future G0–G10 work:

```text
Folder / ZIP / CBZ → Core session → lazy page grid → Core thumbnails
                   → Reader page model / explicit Core-preview fallback
                   → Core-fed CBZ target → progress / cancel → atomic CBZ
```

- The existing book-conversion screen remains intact in the **Books** tab. A
  separate **Comics** tab provides source open/drop, Core-order page grid,
  selected-page source preview and dimensions, warnings, target/output choice,
  progress/cancel, result details and Finder reveal.
- Core/FFI owns Comic and Reader session open/close, summary, stable page IDs,
  page order, selected-page dimensions, thumbnails, preview fallback,
  supported output targets, conversion, progress, cancellation and structured
  errors. Swift transports JSON/Base64 and displays Reader placements/resources
  for supported pages; it does not calculate page geometry or resolve source
  resources.
- The session image cache is LRU-bounded to 128 entries / 32 MiB. The Swift
  thumbnail cache is bounded to 128 entries / 32 MiB. Page tiles use
  `LazyVGrid`; opening a source does not decode every image.
- Preview input is capped at 256 MiB per page, 32,768 pixels per edge and 32
  million source pixels. Decoding is capped at a best-effort 256 MiB decoder
  allocation; generated preview is at most 2,048 pixels per edge, 8 million
  pixels and 16 MiB encoded. Thumbnails are at most 512 pixels per edge.
  Preview output is a PNG display derivative; output CBZ preserves original
  image bytes. Animated GIF preview is a representative still, not an
  animation editor.
- `folio-compat`/Core currently advertises **CBZ only**. Device profile data
  is empty and returned notes state that device-specific preview and
  transformations are not implemented. The GUI does not invent a profile
  picker or imply device emulation.
- The GUI's input boundary is exactly Folder / ZIP / CBZ. EPUB, PDF, 7z/CB7,
  RAR/CBR and other inputs are not accepted through this screen unless a later
  plan gate exposes them. Fixed Layout EPUB remains an existing Core
  capability/probe; it is not silently added to this GUI request.
- CBZ writing consumes the generic Semantic IR image resources, accepts only
  one image node per fixed-layout page, writes deterministic report-free ZIP
  entries through `AtomicFileSink`, reimports the temporary CBZ and byte-
  compares each ordered image resource through IR before commit. Unsupported
  IR structure fails closed. ComicInfo metadata and fixed-layout geometry are
  diagnosed as not embedded.

## Verification and remaining gates

- `cargo check -p folio-core -p folio-ffi` — passed.
- `tests/scripts/run-private-rust-tests.sh` — passed with workspace, private
  integration, Comic model/ingestion, Library checks, formatting and Clippy;
  all generated Cargo targets stayed under the external validation root.
- Private GUI/Core integration harness — 3 passed, 1 local-corpus test
  ignored by default. It covers Folder open/natural order/stable IDs,
  dimensions, Core thumbnail/preview, Core-fed target options, FFI JSON and
  Base64, cancellation, byte-exact IR→CBZ→IR page identity and atomic no-commit
  on cancel.
- Local CBZ corpus flow — passed for both existing comic books (241 and 234
  pages): source hash unchanged, selected page dimensions and Core thumbnail /
  preview passed, CBZ export completed and every output page passed the Core
  IR round-trip validator. Inputs and temporary output were kept outside the
  repository.
- SwiftUI macOS 27 Debug and Release SwiftPM builds — passed against the
  release Rust FFI archive. Existing deprecated `onChange` warnings remain in
  the unrelated 0.1 screens. `packaging/build_macos_app.sh` also passed and
  produced an external arm64 `.app` and ZIP; plist, architecture, no-signature
  and ZIP-integrity checks passed. LaunchServices then rejected the unsigned
  app with `Launchd job spawn failed`. No signing was attempted, so runtime UI
  appearance remains unverified. A bare SwiftPM executable also cannot satisfy
  the existing User Notifications app-bundle requirement.
- One initial real-corpus attempt passed EPUBs directly and Core rejected
  them as outside this GUI scope. The test was corrected to use the two
  already-generated CBZ comparison inputs; no EPUB GUI capability was added.
- Reader/Core-preview A/B now also runs on both real manga EPUBs after the
  existing EPUB IR → CBZ → GUI/Core import path. For each book (241 and 234
  pages), first/middle/last pages preserved the source IR image bytes and
  document order; Reader placement aspect matched the prior Core preview
  within the 0.02 ratio tolerance. This is an image-path regression, not a
  claim of source viewport or spread fidelity.
- One initial private test command created an ignored test-local Cargo target;
  it was moved intact to the external validation root. The first Core check
  also created a repository-root Cargo target before the external target was
  made explicit; that 289 MiB cache was likewise moved intact. All subsequent
  commands explicitly set `CARGO_TARGET_DIR` outside the repository.

The G0–G4 first-path functions listed above now have implementation and test
evidence. Remaining plan work includes broader Core/FFI contract freeze,
unsupported/error-path UI coverage, accessibility and visual acceptance,
large-corpus performance acceptance, device profiles, edits, target preview,
additional outputs, and all dependent GUI features. C3 remains unaccepted; no
GUI behavior claims RAR/7z/PDF/EPUB input support. No GitHub push was made.

## Verification record

Implementation and validation entries are appended to the ignored
`.folioforge-dev/DEVELOPMENT_LOG.md` and summarized in ignored
`.codex/STATUS.md`. G5–G10 are not implied complete by the first usable path.
