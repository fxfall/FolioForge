# Changelog

## 0.2.1 development — Reader Runtime

- Completed the Preview A audit and captured synthetic EPUB, Markdown and HTML
  snapshots locally. Added the initial format-blind `folio-reader` session,
  location, viewport and typed-error foundation over generic Folio IR. R2 adds
  one resource resolver over `Book::load_resource`; R3 adds strict fixed-layout
  single-image-page rendering, geometry, Fit/Fill/Actual Size, direction-aware
  navigation and zoom/pan. The initial Comic import→IR→Reader image path is
  covered by private local tests. R4 adds a Core-owned session lifecycle and
  FFI transport for fixed-page models, resources, navigation and viewport
  updates, including reuse of an already-open Comic IR session. R5 connects the
  SwiftUI Comic canvas to Reader render models for supported fixed single-image
  pages, with an explicit Core-preview fallback at typed support/transport
  boundaries. Both local real manga books passed first/middle/last image-path
  A/B, and macOS 27 Debug/Release builds pass. This is a major additive 0.2.1
  architecture/API milestone; it does not change the 0.1 release line.
  R6 moves format-blind reflowable IR rendering to `folio-reader` while Core
  retains edit/target-compatibility projection and the exact same response DTO.
  Preview A/B is byte-identical for simple/rich EPUB, Markdown and HTML. The old
  crate is no longer a Core runtime dependency and remains only as the local A
  oracle. R7 adds generic Reader navigation and exact IR TOC/landmark/page-list
  targets, with typed unresolved destinations and no guessed redirects. R8 adds
  generic EPUB-backed spread intent and opt-in Reader spread geometry without
  format-specific inference. R9 adds a source/edited/target Reader Preview API
  and fixed-page FFI output while Core retains input, edit and compatibility
  authority; Target preview matches the Preview A DTO and HTML byte-for-byte.
  R10 profiles both real 241/234-page manga samples, adds bounded
  session-scoped AppKit image reuse, and verifies no full-comic decode. R11
  migrates the SwiftUI compatibility preview and inspector to the Reader FFI;
  R12 removes the duplicate `folio-preview` renderer and redundant Swift DTOs.
  The frozen 0.1 Core/FFI/Service preview contracts remain thin adapters to
  Reader. Four local Preview A goldens match Reader DTO/HTML output, the
  501-page spread-navigation index regression passes, and macOS 27 arm64
  Debug/Release SwiftPM builds pass. The full private Rust regression,
  Clippy, formatting, architecture and C-header checks pass; private corpus
  tests remain local and are not part of public artifacts.

## 0.2.0 development — Comic/Manga Core

- Froze the comic Core boundaries, source/edit/render ownership, feature gates,
  KCC comparison protocol and initial Core audit in `docs/0.2/`. This is
  additive to the frozen 0.1 release.
- Added the C2 Rust native-source model and deterministic page ordering. This
  does not yet add comic input, analysis, editing, image processing or output.
  C2 passed its local regression, workspace, architecture, compatibility and
  cross-client parity gates, plus three pinned KCC 11.3.2 ordering probes.
  C3 ingestion is in development: bounded directory, ZIP/CBZ, constrained
  7z/CB7, classic-xref PDF catalog and eligible FixedLayout EPUB adapters are
  implemented. RAR/CBR, unsupported PDF/7z variants and PDF rasterization
  remain deferred; C3 is not accepted. EPUB import safely accepts the exact
  external DAISY 2005 NCX DTD without fetching it, while still rejecting other
  DTD declarations and internal subsets. An additive Comic 0.2 API now projects
  supported raster sources to a lazy, source-order Semantic IR representation;
  Fixed Layout EPUB reuses its EPUB IR. Viewport, spread/side, manga direction
  and alt-text semantics remain explicit gaps, so C6/C7 are not complete and
  no rendering-fidelity claim is made. Added a separate SwiftUI Comics tab
  for Folder/ZIP/CBZ sources with Core-owned page sessions, lazy thumbnails,
  source preview, CBZ-only IR-backed output, progress and cancellation. This is
  an unreleased 0.2 major API/UI addition; the 0.1 book workflow is unchanged.

## 0.1.0 release infrastructure

- Added Phase 7.7 GitHub Actions definitions for native Linux and macOS 27
  Core builds plus the separate SwiftUI GUI artifact.
- Removed the unvalidated Windows packaging path from the frozen
  0.1 public artifact matrix; tagged releases now publish four archives.
- Added platform smoke packaging, Unicode-path conversion checks, semantic
  parity checks, Library-database exclusion checks and tagged SHA256 release
  assembly. No conversion behavior or public API changed.
- Separated maintainer-only Python/oracle tooling, synthetic validation
  fixtures, benchmarks and development records into the ignored
  `.folioforge-dev/` bundle. Public packaging helpers now live under
  `packaging/`; no conversion behavior or public API changed.

## 0.1.0

- Rust Semantic IR conversion core with EPUB, Kindle-family, text, Markdown,
  HTML/HTMLZ, FB2 and DOCX import paths.
- EPUB, KF7, KF8, KF7+KF8 compatibility-container and KFX writer targets.
- DRM-free reflowable KFX input recovery with explicit diagnostics and
  unsupported-feature reporting; protected content is rejected without
  decryption.
- Deterministic output, compatibility planning, validation, progress events,
  atomic file writes and batch conversion.
- C FFI, local HTTP service and SwiftUI macOS client surfaces over the same
  Core contracts.
- Optional `folio-library` data foundation with SQLite migrations, scanning,
  metadata inspection, FTS search and conversion provenance.

This release does not claim fixed-layout KFX, DRM decryption, Kindle device
equivalence, notarized macOS distribution or cloud synchronization.
