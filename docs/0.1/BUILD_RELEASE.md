# FolioForge 0.1 Build and Release

## Prerequisites

- Rust stable with `cargo`, `rustfmt` and `clippy`.
- Python 3 for public scripts and the architecture/repository gates.
- macOS 13 or newer, Swift 5.9 or newer and Xcode command-line tools for the
  SwiftUI package and macOS arm64 packaging.
- Docker only for the optional Service image smoke test.

Calibre, Bōkō and Kindle Previewer are private research references. They are
not required to build or test a clean public checkout.

## External validation root

Set `FOLIOFORGE_VALIDATION_ROOT` to a writable directory outside this
repository before local builds that generate artifacts. A release script may
derive a run-specific child directory from it. Do not commit, archive or
document a machine-specific absolute path.

```bash
export FOLIOFORGE_VALIDATION_ROOT="$VALIDATION_VOLUME/folioforge-0.1"
mkdir -p "$FOLIOFORGE_VALIDATION_ROOT"
export CARGO_TARGET_DIR="$FOLIOFORGE_VALIDATION_ROOT/cargo-target"
export TMPDIR="$FOLIOFORGE_VALIDATION_ROOT/tmp"
export FOLIOFORGE_TEMP_ROOT="$FOLIOFORGE_VALIDATION_ROOT/runtime"
```

## Rust checks

```bash
cargo fmt --all -- --check
python3 tests/architecture/check_boundaries.py
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release
```

The public scripts use only synthetic fixtures. Their generated files must be
passed an external output directory.

## Service and CLI smoke

```bash
tests/scripts/build_minimal_corpus.sh "$FOLIOFORGE_VALIDATION_ROOT/minimal"
FOLIO_BIN="$CARGO_TARGET_DIR/release/folio" \
  tests/scripts/run_conversion_matrix.sh \
  "$FOLIOFORGE_VALIDATION_ROOT/minimal/001-text.epub"
FOLIO_BIN="$CARGO_TARGET_DIR/release/folio" \
FOLIO_SERVICE_BIN="$CARGO_TARGET_DIR/release/folio-service" \
  tests/scripts/run_service_smoke.sh
```

## SwiftUI debug/release

Build the Rust FFI for the host/target pair, then pass its static archive to
SwiftPM:

```bash
cargo build --locked --release -p folio-ffi
FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a" \
  swift build --package-path macos/FolioForge \
  --scratch-path "$FOLIOFORGE_VALIDATION_ROOT/swift-debug"
FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a" \
  swift build --package-path macos/FolioForge \
  --scratch-path "$FOLIOFORGE_VALIDATION_ROOT/swift-release" -c release
```

The arm64 packaging entry point is `tests/scripts/build_macos_app.sh`. It
checks Rust format/tests/clippy, builds the macOS 13 arm64 FFI and SwiftUI
release, creates the checked-in logo icon, validates the intentionally unsigned
app/ZIP and writes the timestamped product under ignored `dist/`. It does not
invoke `codesign`, use certificates or perform notarization.

## Docker Service

```bash
docker build --build-arg FOLIOFORGE_VERSION=0.1.0 \
  -t folioforge:0.1.0 .
docker run --rm -p 127.0.0.1:8080:8080 \
  -v "$FOLIOFORGE_VALIDATION_ROOT/service-work:/work" \
  folioforge:0.1.0
```

## Repository clean gate

Before a commit or push, preview ignored files with `git clean -ndX`. Review
the list; do not run a blind `git clean -fdx`. The checked-in repository audit
is `python3 tools/release/repository-audit.py`; it fails for tracked private
corpora, generated output, local paths, high-signal secrets or oversized files.

## Release sequence

1. Keep the working tree clean and record the 0.1 finalization base commit.
2. Run the complete local validation and update `docs/0.1/AUDIT.md`.
3. Push `main` to `https://github.com/fxfall/FolioForge` only after inspecting
   the remote; never force-push an unknown history.
4. Let `ci.yml` pass on a clean checkout.
5. Build and test `0.1.0-rc.1` from a fresh clone and tag it only after CI.
6. Promote the verified commit to `v0.1.0` and publish only artifacts that
   were actually built and validated.
