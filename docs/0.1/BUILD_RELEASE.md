# FolioForge 0.1 Build and Release

## Prerequisites

- Rust stable with `cargo`, `rustfmt` and `clippy`.
- macOS 27 or newer, Swift 6.4 or newer and Xcode 27 command-line tools for the
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
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked --release
```

The extended synthetic/oracle validation bundle is local-only. When present,
run it from `.folioforge-dev/` and keep all generated files under the external
`FOLIOFORGE_VALIDATION_ROOT`.

## Optional maintainer-local Service and CLI smoke

```bash
.folioforge-dev/tests/scripts/build_minimal_corpus.sh "$FOLIOFORGE_VALIDATION_ROOT/minimal"
FOLIO_BIN="$CARGO_TARGET_DIR/release/folio" \
  .folioforge-dev/tests/scripts/run_conversion_matrix.sh \
  "$FOLIOFORGE_VALIDATION_ROOT/minimal/001-text.epub"
FOLIO_BIN="$CARGO_TARGET_DIR/release/folio" \
FOLIO_SERVICE_BIN="$CARGO_TARGET_DIR/release/folio-service" \
  .folioforge-dev/tests/scripts/run_service_smoke.sh
```

These scripts are not available in a clean public checkout by design.

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

The arm64 packaging entry point is `packaging/build_macos_app.sh`. It
checks Rust format/tests/clippy, builds the macOS 27 arm64 FFI and SwiftUI
release, creates the checked-in logo icon, validates the intentionally unsigned
app/ZIP and writes the timestamped product under ignored `dist/`. It does not
use certificates or perform notarization; it also strips any ad-hoc signature
automatically emitted by the Apple linker.

For a host-native package on a newer macOS SDK, set
`FOLIOFORGE_MACOS_TARGET_VERSION` before invoking the same script. For example,
the macOS 27 package used for local and hosted validation is built with
`FOLIOFORGE_MACOS_TARGET_VERSION=27.0`; the script passes that target to SwiftPM
and writes the matching minimum system version into the staged app bundle. The
default is the public macOS 27 release floor; an older deployment target is not
part of the 0.1 release contract.

## GitHub Actions Phase 7.7

The multi-platform release definition is recorded in
[PHASE_7_7_CI.md](PHASE_7_7_CI.md). The public workflow set is deliberately
limited to `ci.yml`, `build.yml` and `release.yml`:

- `ci.yml` is the Rust source-quality gate and produces no release archive.
- `build.yml` builds native Linux x86_64/ARM64, Windows x86_64/ARM64 and
  macOS 27 ARM64 Core/GUI artifacts on `main` or manual dispatch.
- `release.yml` rebuilds the same six artifacts from a `v*` tag, creates
  `SHA256SUMS`, and publishes only those artifacts.

The GitHub release build does not sign macOS output. The GUI job uses
`xcode-27`, sets the minimum target to `27.0`, checks the self-contained bundle
and rejects repository or private Homebrew dynamic dependencies. Linux and
Windows ARM jobs execute their own binaries on native ARM runners. The hosted
runner labels are intentionally explicit because the 26/27 images are a
moving hosted-image boundary; changing a future label must not change the Core
or GUI packaging contract.

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
the list; do not run a blind `git clean -fdx`. The public repository clean
gate verifies that the maintainer-only `.folioforge-dev/` tree and its
historical `tests/`, `tools/` and `bench/` paths are not tracked. Full private
corpus and provenance audits remain in the local development record and are
never release inputs.

## Release sequence

1. Keep the working tree clean and record the 0.1 finalization base commit.
2. Run the complete local validation and update `docs/0.1/AUDIT.md`.
3. Push `main` to `https://github.com/fxfall/FolioForge` only after inspecting
   the remote; never force-push an unknown history.
4. Let `ci.yml` pass on a clean checkout.
5. Build and test `0.1.0-rc.1` from a fresh clone and tag it only after CI.
6. Promote the verified commit to `v0.1.0` and publish only artifacts that
   were actually built and validated.
