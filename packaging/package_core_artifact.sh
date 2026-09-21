#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
    printf '%s\n' "usage: package_core_artifact.sh RUST_TARGET ARTIFACT_LABEL ARTIFACT_ROOT" >&2
    exit 2
fi

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
RUST_TARGET=$1
ARTIFACT_LABEL=$2
ARTIFACT_ROOT=$3
VALIDATION_ROOT=${FOLIOFORGE_VALIDATION_ROOT:?set FOLIOFORGE_VALIDATION_ROOT to an external writable directory}

case "$RUST_TARGET" in
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu|aarch64-apple-darwin)
        ;;
    *)
        printf '%s\n' "unsupported Unix Core target: $RUST_TARGET" >&2
        exit 2
        ;;
esac

case "$ARTIFACT_LABEL" in
    linux-x86_64|linux-aarch64|macos27-aarch64)
        ;;
    *)
        printf '%s\n' "unsupported Core artifact label: $ARTIFACT_LABEL" >&2
        exit 2
        ;;
esac

BUILD_STAMP=$(date '+%Y%m%d-%H%M%S')
BUILD_ROOT=${FOLIOFORGE_CORE_BUILD_ROOT:-$VALIDATION_ROOT/core-$ARTIFACT_LABEL-$BUILD_STAMP}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$BUILD_ROOT/cargo-target}
TMPDIR=${TMPDIR:-$BUILD_ROOT/tmp}
FOLIOFORGE_TEMP_ROOT=${FOLIOFORGE_TEMP_ROOT:-$BUILD_ROOT/runtime}
export CARGO_TARGET_DIR TMPDIR FOLIOFORGE_TEMP_ROOT
mkdir -p "$BUILD_ROOT" "$CARGO_TARGET_DIR" "$TMPDIR" "$FOLIOFORGE_TEMP_ROOT" "$ARTIFACT_ROOT"

cleanup() {
    exit_code=$?
    if [ -d "$BUILD_ROOT" ]; then
        rm -rf "$BUILD_ROOT"
    fi
    exit "$exit_code"
}
trap cleanup EXIT HUP INT TERM

rustup target add "$RUST_TARGET"
cargo test --locked -p folio-cli -p folio-core
cargo build --locked --release -p folio-cli --target "$RUST_TARGET"

BINARY="$CARGO_TARGET_DIR/$RUST_TARGET/release/folio"
test -x "$BINARY"
case "$ARTIFACT_LABEL" in
    linux-x86_64)
        file "$BINARY" | grep -Eq 'x86-64|x86_64'
        ;;
    linux-aarch64|macos27-aarch64)
        file "$BINARY" | grep -Eq 'aarch64|arm64'
        ;;
esac

VERSION_OUTPUT=$("$BINARY" --version)
printf '%s\n' "$VERSION_OUTPUT"
printf '%s\n' "$VERSION_OUTPUT" | grep -q '0\.1\.0'
"$BINARY" --help >/dev/null

if find "$BUILD_ROOT" -type f \( -name 'library.sqlite' -o -name '*.sqlite' -o -name '*.sqlite3' \) -print -quit | grep -q .; then
    printf '%s\n' 'Core build unexpectedly created a Library database' >&2
    exit 1
fi

PACKAGE_NAME="FolioForge-Core-0.1.0-$ARTIFACT_LABEL"
PACKAGE_ROOT="$BUILD_ROOT/package/$PACKAGE_NAME"
ARCHIVE="$ARTIFACT_ROOT/$PACKAGE_NAME.tar.gz"
mkdir -p "$PACKAGE_ROOT/bin"
cp "$BINARY" "$PACKAGE_ROOT/bin/folio"
cp "$ROOT_DIR/LICENSE" "$PACKAGE_ROOT/LICENSE"
chmod +x "$PACKAGE_ROOT/bin/folio"

tar -C "$BUILD_ROOT/package" -czf "$ARCHIVE" "$PACKAGE_NAME"
tar -tzf "$ARCHIVE" | grep -qx "$PACKAGE_NAME/bin/folio"
tar -tzf "$ARCHIVE" | grep -qx "$PACKAGE_NAME/LICENSE"
if tar -tzf "$ARCHIVE" | grep -Eq '(^|/)(library\.sqlite|.*\.sqlite3?)$'; then
    printf '%s\n' 'Core archive contains a Library database' >&2
    exit 1
fi

printf '%s\n' "Built and packaged $ARCHIVE"
