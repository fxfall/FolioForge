#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT_DIR"

HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
    arm64|x86_64)
        ;;
    *)
        printf '%s\n' "Unsupported macOS host architecture: $HOST_ARCH" >&2
        exit 2
        ;;
esac

MARKETING_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' packaging/FolioForge-Info.plist)
BUNDLE_BUILD_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' packaging/FolioForge-Info.plist)
if [ "$MARKETING_VERSION" != "0.1.0" ]; then
    printf '%s\n' "Expected FolioForge marketing version 0.1.0, found $MARKETING_VERSION" >&2
    exit 2
fi
case "$BUNDLE_BUILD_VERSION" in
    ''|*[!0-9]*)
        printf '%s\n' "CFBundleVersion must be a positive integer, found $BUNDLE_BUILD_VERSION" >&2
        exit 2
        ;;
esac
if [ "$BUNDLE_BUILD_VERSION" -lt 1 ]; then
    printf '%s\n' "CFBundleVersion must be positive, found $BUNDLE_BUILD_VERSION" >&2
    exit 2
fi

# Keep all compiler caches, Swift scratch data, runtime scratch data and app
# staging outside the repository. A unique default root also prevents an old
# build from being mistaken for the current validation result.
BUILD_STAMP=$(date '+%Y%m%d-%H%M%S')
VALIDATION_ROOT=${FOLIOFORGE_VALIDATION_ROOT:?set FOLIOFORGE_VALIDATION_ROOT to an external writable directory}
BUILD_ROOT=${FOLIOFORGE_PHASE7_BUILD_ROOT:-$VALIDATION_ROOT/macos-build-$BUILD_STAMP}
CARGO_TARGET_DIR="$BUILD_ROOT/cargo-target"
TMPDIR="$BUILD_ROOT/tmp"
FOLIOFORGE_TEMP_ROOT="$BUILD_ROOT/runtime"
SWIFT_BUILD_ROOT="$BUILD_ROOT/swift-build"
export CARGO_TARGET_DIR TMPDIR FOLIOFORGE_TEMP_ROOT
mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR" "$FOLIOFORGE_TEMP_ROOT"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

# The release artifact is always Apple Silicon. Use the native arm64 build
# path on Apple Silicon runners (the same path exercised by the macOS CI job).
# Keep an explicit cross target only for an Intel fallback runner.
if [ "$HOST_ARCH" = "arm64" ]; then
    MACOSX_DEPLOYMENT_TARGET=13.0 \
        cargo build --locked --release -p folio-ffi
    FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a"
else
    rustup target add aarch64-apple-darwin
    CFLAGS_aarch64_apple_darwin='-mmacosx-version-min=13.0' \
    RUSTC_WRAPPER="$ROOT_DIR/tests/scripts/rustc_macos_target_wrapper.sh" \
        cargo build --locked --target aarch64-apple-darwin --release -p folio-ffi
    FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/libfolio_ffi.a"
fi

SWIFT_TARGET_TRIPLE=arm64-apple-macosx13.0
FOLIOFORGE_FFI_ARCHIVE="$FOLIOFORGE_FFI_ARCHIVE" \
    swift build --package-path macos/FolioForge --scratch-path "$SWIFT_BUILD_ROOT" \
        --triple "$SWIFT_TARGET_TRIPLE" -c release

PRODUCT_DIR=$(FOLIOFORGE_FFI_ARCHIVE="$FOLIOFORGE_FFI_ARCHIVE" \
    swift build --package-path macos/FolioForge --scratch-path "$SWIFT_BUILD_ROOT" \
        --triple "$SWIFT_TARGET_TRIPLE" -c release --show-bin-path)
DIST_DIR="$ROOT_DIR/dist"
BUILD_BASENAME="FolioForge-macOS-arm64-0.1.0-$BUILD_STAMP"
FINAL_DIR="$DIST_DIR/$BUILD_BASENAME"
FINAL_APP="$FINAL_DIR/FolioForge.app"
FINAL_ZIP="$DIST_DIR/$BUILD_BASENAME.zip"
STAGING=$(mktemp -d "$BUILD_ROOT/staging.XXXXXX")
trap 'rm -rf "$STAGING"' EXIT HUP INT TERM

if [ -e "$FINAL_DIR" ] || [ -e "$FINAL_ZIP" ]; then
    printf '%s\n' "Refusing to overwrite an existing build: $BUILD_BASENAME" >&2
    exit 2
fi

APP="$STAGING/FolioForge.app"
CONTENTS="$APP/Contents"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$PRODUCT_DIR/FolioForge" "$CONTENTS/MacOS/FolioForge"
cp -R "$PRODUCT_DIR/FolioForge_FolioForge.bundle" "$CONTENTS/Resources/"
cp packaging/FolioForge-Info.plist "$CONTENTS/Info.plist"

ICONSET="$STAGING/FolioForge.iconset"
mkdir -p "$ICONSET"
LOGO="$ROOT_DIR/assets/folioforge-logo.png"
sips -z 16 16 "$LOGO" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32 "$LOGO" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32 "$LOGO" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64 "$LOGO" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128 "$LOGO" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256 "$LOGO" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$LOGO" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512 "$LOGO" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$LOGO" --out "$ICONSET/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$LOGO" --out "$ICONSET/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$CONTENTS/Resources/FolioForge.icns"

plutil -lint "$CONTENTS/Info.plist"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$CONTENTS/Info.plist")" = "$MARKETING_VERSION"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$CONTENTS/Info.plist")" = "$BUNDLE_BUILD_VERSION"
codesign --force --deep --sign - --entitlements "$ROOT_DIR/macos/FolioForge/FolioForge.entitlements" "$APP"
codesign --verify --deep --strict "$APP"
SIGNED_ENTITLEMENTS=$(codesign -d --entitlements :- "$APP" 2>/dev/null)
printf '%s\n' "$SIGNED_ENTITLEMENTS" | grep -q '<key>com.apple.security.app-sandbox</key>'
printf '%s\n' "$SIGNED_ENTITLEMENTS" | grep -q '<key>com.apple.security.files.user-selected.read-write</key>'
printf '%s\n' "$SIGNED_ENTITLEMENTS" | grep -q '<key>com.apple.security.network.client</key>'
test -x "$CONTENTS/MacOS/FolioForge"
test -f "$CONTENTS/Resources/FolioForge_FolioForge.bundle/Contents/Resources/folioforge-logo.png"
test -f "$CONTENTS/Resources/FolioForge.icns"
file "$CONTENTS/MacOS/FolioForge" | grep -q 'arm64'

cp macos/FolioForge/README.md "$STAGING/README.md"
ditto -c -k --sequesterRsrc --keepParent "$STAGING/FolioForge.app" "$STAGING/FolioForge-macOS-arm64-0.1.0.zip"
unzip -t "$STAGING/FolioForge-macOS-arm64-0.1.0.zip"

# Publish to this unique build path only after validation passed.
mkdir -p "$FINAL_DIR"
cp -R "$STAGING/FolioForge.app" "$FINAL_DIR/FolioForge.app"
cp "$STAGING/README.md" "$FINAL_DIR/README.md"
cp "$STAGING/FolioForge-macOS-arm64-0.1.0.zip" "$FINAL_ZIP"

printf '%s\n' "Built and validated: $FINAL_APP" "$FINAL_ZIP"
