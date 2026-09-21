# Changelog

## 0.1.0 release infrastructure

- Added Phase 7.7 GitHub Actions definitions for native Linux, Windows and
  macOS 27 Core builds plus the separate SwiftUI GUI artifact.
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
