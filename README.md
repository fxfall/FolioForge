# FolioForge 0.3.1

![FolioForge logo](assets/folioforge-logo.png)

FolioForge is an offline-first Rust book conversion and comic/manga preparation
application. The Core, command-line tools, native interfaces, and desktop app
use the same product version and share the same conversion and capability
contracts.

Download the [latest GitHub release](https://github.com/fxfall/FolioForge/releases/latest).
Release notes and supported desktop packages are listed in
[docs/0.3/RELEASE.md](docs/0.3/RELEASE.md).

## Features

- Convert EPUB, MOBI/KF7, AZW3/KF8, DRM-free reflowable KFX, TXT, Markdown,
  HTML/HTMLZ, FB2 and DOCX through one Semantic IR pipeline.
- Export EPUB3, KF7, KF8, KF7+KF8 compatibility containers and FolioForge's
  explicit KFX compatibility container.
- Preserve or diagnose navigation, links, notes, images, styles, fonts, ruby,
  MathML and layout semantics according to source evidence and target support.
- Import supported image folders and ZIP/CBZ comic sources, browse ordered
  pages, preview them in Reader, and export CBZ.
- Inspect compatibility and diagnostics before conversion; choose Strict,
  Compatible or Readable degradation behavior; process batches with progress
  and cancellation.
- Use English or Simplified Chinese in the desktop interface.
- Convert locally without network access. Optional metadata lookup is a
  separate action and never uploads a book.

Comic import is limited to supported image folders and ZIP/CBZ page sources;
CBZ is the current comic output. DRM decryption, fixed-layout KFX, and
unsupported comic transformations are not claimed.

## Quick start

```bash
cargo build --locked --release -p folio-cli
target/release/folio capabilities
target/release/folio convert input.epub --to epub --output output.epub
target/release/folio validate output.epub
```

The CLI also provides `analyze` and `inspect`. The C FFI and local HTTP service
use the same Core APIs and product version.

## Desktop downloads

The current release provides Apple Silicon macOS, Linux x86_64/ARM64, and
Windows x86_64/ARM64 packages. GitHub macOS packages are unsigned and
unnotarized. See the release notes for installation and runtime requirements.

## Build

Use Rust stable for Core and CLI builds. macOS desktop packaging requires
macOS 27+, Swift 6.4+, and Xcode 27. Build and scratch data must be kept
outside the repository under a directory selected by
`FOLIOFORGE_VALIDATION_ROOT`.

Current product version information and user-visible changes are documented in
[CHANGELOG.md](CHANGELOG.md). Historical format and data contracts remain under
`docs/`.
