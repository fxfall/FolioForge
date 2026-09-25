# FolioForge desktop application

![FolioForge logo](../../assets/folioforge-logo.png)

Product version `0.3.1` is shared with the Rust workspace, Core, and FFI. The
application provides local book conversion, compatibility checks, batch
progress and cancellation, diagnostics, metadata/style editing, Reader preview,
and a supported comic workspace for image folders and ZIP/CBZ sources.

Conversion and format decisions come from Core. Local conversion does not
require network access; optional metadata lookup sends search terms only and
requires a user confirmation before applying changes.

Comic support is intentionally limited to the input and output subset listed
in [the feature matrix](../../docs/0.2/COMIC_FEATURE_MATRIX.md). The current
desktop comic workflow provides ordered page browsing, thumbnails, preview and
CBZ output; it does not claim general PDF, RAR, fixed-layout KFX, or comic
editing support.

## Build

From the repository root, build the Rust FFI archive and pass it to SwiftPM:

```bash
export FOLIOFORGE_VALIDATION_ROOT="$VALIDATION_VOLUME/folioforge-0.3.1"
export CARGO_TARGET_DIR="$FOLIOFORGE_VALIDATION_ROOT/cargo-target"
cargo build --locked --release -p folio-ffi
FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a" \
  swift build --package-path macos/FolioForge \
  --scratch-path "$FOLIOFORGE_VALIDATION_ROOT/swift-debug"
FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a" \
  swift build --package-path macos/FolioForge \
  --scratch-path "$FOLIOFORGE_VALIDATION_ROOT/swift-release" -c release
```

`packaging/build_macos_app.sh` is the complete Apple Silicon package
validation entry point. It requires `FOLIOFORGE_VALIDATION_ROOT`, keeps build
scratch outside the repository, builds against the macOS 27 floor, embeds the
project logo, and rejects an app/Core version mismatch. Local output defaults
to the version in the app bundle metadata.

The local package is signed only when a valid local keychain identity is
available. GitHub packages are intentionally unsigned and not notarized; no
local signing identity or certificate is used by GitHub Actions.
