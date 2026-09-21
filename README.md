# FolioForge 0.1

![FolioForge logo](assets/folioforge-logo.png)

FolioForge is a local, Rust-based book importer and converter. It imports
multiple ebook and rich-document formats into one Semantic IR, applies an
explicit compatibility plan, and writes a validated output artifact. The
SwiftUI macOS client, C FFI and local HTTP service all call this same Core
pipeline.

## 0.1 capabilities

- Import EPUB, KF7/MOBI, KF8/AZW3, KFX, TXT, Markdown, HTML/HTMLZ, FB2 and
  DOCX/OOXML.
- Export EPUB3, KF7, KF8, KF7+KF8 compatibility containers and FolioForge's
  explicit KFX compatibility writer output.
- Preserve or report structure, navigation, links, notes, images, SVG,
  styles, fonts, ruby, MathML and layout intent through the Semantic IR where
  the input and target can prove the relationship.
- Use deterministic IDs and ordering by default, with target diagnostics and
  round-trip validation in every conversion report.
- Run offline by default. Online metadata lookup is an explicit service/client
  action and never receives a book file.

## Quick start

```bash
cargo build --locked --release -p folio-cli
target/release/folio capabilities
target/release/folio convert input.epub --to epub --output output.epub
target/release/folio validate output.epub
```

The CLI also provides `analyze` and `inspect`. `--mode strict` rejects
unrepresentable target features; `compatible` records target-safe fallbacks;
`readable` permits the most user-visible approximation. The complete command
and JSON contract is in [docs/0.1/API.md](docs/0.1/API.md).

## Clients

- macOS: `macos/FolioForge` is a Swift Package executable. It owns file
  selection, queue state, editing controls and preview presentation; it does
  not parse formats or invent compatibility decisions.
- Service: `folio-service` is a local HTTP adapter with bounded uploads,
  isolated work directories, progress events and downloadable reports.
- FFI: `folio-ffi` exposes versioned JSON requests/reports and cancellation to
  the macOS client and other native hosts.
- Library: `folio-library` is optional metadata/index storage. It is not part
  of standalone conversion and does not store Semantic IR or resource BLOBs.

## Build

Use the prerequisites and release commands in
[docs/0.1/BUILD_RELEASE.md](docs/0.1/BUILD_RELEASE.md). Build and benchmark
scratch data must be placed in a directory selected by
`FOLIOFORGE_VALIDATION_ROOT`, not in the repository.

The checked-in macOS packaging script creates an intentionally unsigned
development app. It does not invoke `codesign`, use certificates, claim
Developer ID signing or notarization, or claim Kindle device equivalence.

## Support boundaries

KFX input support is for DRM-free, reflowable books and is diagnostic-first:
unproven relationships remain unresolved and protected content is rejected.
Fixed-layout/comic KFX, DRM decryption, proprietary online services and
device-specific rendering are outside this release. See
[docs/0.1/FORMAT_SUPPORT.md](docs/0.1/FORMAT_SUPPORT.md) and
[docs/0.1/KNOWN_LIMITS.md](docs/0.1/KNOWN_LIMITS.md).

## Documentation

The current 0.1 documentation is the only authoritative development contract:

1. [DEVELOPMENT.md](docs/0.1/DEVELOPMENT.md)
2. [API.md](docs/0.1/API.md)
3. the relevant [format](docs/formats/) or [Library](docs/library/DATA_MODEL.md)
   contract
4. [KNOWN_LIMITS.md](docs/0.1/KNOWN_LIMITS.md)
5. source-level Rust tests; maintainer-only validation material is kept in the
   ignored local `.folioforge-dev/` directory and is not part of GitHub

Historical stage notes, Python/oracle tools, synthetic validation fixtures and
development logs are retained in the ignored local `.folioforge-dev/` bundle;
they are not authoritative runtime code and are not published to GitHub.
