#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT_DIR"
PHASE=host-validation
STAGING=

cleanup() {
    exit_code=$?
    if [ -n "$STAGING" ] && [ -e "$STAGING" ]; then
        rm -rf "$STAGING"
    fi
    if [ "$exit_code" -ne 0 ]; then
        printf '::error title=FolioForge macOS packaging::phase=%s exit=%s\n' \
            "$PHASE" "$exit_code" >&2
    fi
    exit "$exit_code"
}
trap cleanup EXIT HUP INT TERM

HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
    arm64|x86_64)
        ;;
    *)
        printf '%s\n' "Unsupported macOS host architecture: $HOST_ARCH" >&2
        exit 2
        ;;
esac

PHASE=version-validation
BASE_MARKETING_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' packaging/FolioForge-Info.plist)
MARKETING_VERSION=${FOLIOFORGE_APP_VERSION:-$BASE_MARKETING_VERSION}
BUNDLE_BUILD_VERSION=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' packaging/FolioForge-Info.plist)
if [ "$BASE_MARKETING_VERSION" != "0.1.0" ]; then
    printf '%s\n' "Expected the frozen base bundle version 0.1.0, found $BASE_MARKETING_VERSION" >&2
    exit 2
fi
if ! printf '%s\n' "$MARKETING_VERSION" | grep -Eq '^[0-9]+(\.[0-9]+){2}$'; then
    printf '%s\n' "FOLIOFORGE_APP_VERSION must be a numeric three-part version, found $MARKETING_VERSION" >&2
    exit 2
fi
case "$BUNDLE_BUILD_VERSION" in
    ''|*[!0-9]*)
        printf '%s\n' "CFBundleVersion must be a positive integer, found $BUNDLE_BUILD_VERSION" >&2
        exit 2
        ;;
esac

# Local packaging signs with an available keychain identity by default.
# Hosted public workflows must explicitly set this to "none" so a runner
# can never sign or publish with an imported maintainer certificate.
CODESIGN_REQUEST=${FOLIOFORGE_CODESIGN_IDENTITY:-auto}
SIGNING_IDENTITY=
case "$CODESIGN_REQUEST" in
    auto)
        DEVELOPER_ID_IDENTITIES=$(security find-identity -v -p codesigning 2>/dev/null \
            | awk '/^[[:space:]]*[0-9]+\)/ && /Developer ID Application:/ { print $2 }')
        APPLE_DEV_IDENTITIES=$(security find-identity -v -p codesigning 2>/dev/null \
            | awk '/^[[:space:]]*[0-9]+\)/ && /Apple Development:/ { print $2 }')
        DEVELOPER_ID_COUNT=$(printf '%s\n' "$DEVELOPER_ID_IDENTITIES" | awk 'NF { n++ } END { print n + 0 }')
        APPLE_DEV_COUNT=$(printf '%s\n' "$APPLE_DEV_IDENTITIES" | awk 'NF { n++ } END { print n + 0 }')
        if [ "$DEVELOPER_ID_COUNT" -eq 1 ]; then
            SIGNING_IDENTITY=$DEVELOPER_ID_IDENTITIES
        elif [ "$DEVELOPER_ID_COUNT" -gt 1 ]; then
            printf '%s\n' "Multiple Developer ID identities found; set FOLIOFORGE_CODESIGN_IDENTITY explicitly" >&2
            exit 2
        elif [ "$APPLE_DEV_COUNT" -eq 1 ]; then
            SIGNING_IDENTITY=$APPLE_DEV_IDENTITIES
        elif [ "$APPLE_DEV_COUNT" -gt 1 ]; then
            printf '%s\n' "Multiple Apple Development identities found; set FOLIOFORGE_CODESIGN_IDENTITY explicitly" >&2
            exit 2
        else
            printf '%s\n' "No valid local signing identity found; set FOLIOFORGE_CODESIGN_IDENTITY=none only for an intentionally unsigned build" >&2
            exit 2
        fi
        ;;
    none|unsigned)
        ;;
    -)
        printf '%s\n' "Ad-hoc signing is not supported; select a keychain identity or use none for an unsigned build" >&2
        exit 2
        ;;
    *)
        SIGNING_IDENTITY=$CODESIGN_REQUEST
        ;;
esac
if [ -n "$SIGNING_IDENTITY" ]; then
    SIGNING_MODE=signed
else
    SIGNING_MODE=unsigned
fi
if [ "$BUNDLE_BUILD_VERSION" -lt 1 ]; then
    printf '%s\n' "CFBundleVersion must be positive, found $BUNDLE_BUILD_VERSION" >&2
    exit 2
fi

MACOS_TARGET_VERSION=${FOLIOFORGE_MACOS_TARGET_VERSION:-27.0}
case "$MACOS_TARGET_VERSION" in
    [0-9]*.[0-9]*)
        ;;
    *)
        printf '%s\n' "FOLIOFORGE_MACOS_TARGET_VERSION must be a dotted macOS 27 target, found $MACOS_TARGET_VERSION" >&2
        exit 2
        ;;
esac

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

PHASE=rust-format
cargo fmt --all -- --check
PHASE=rust-clippy
cargo clippy --workspace --lib --bins --locked -- -D warnings

# The release artifact is always Apple Silicon. Use the native arm64 build
# path on Apple Silicon runners (the same path exercised by the macOS CI job).
# Keep an explicit cross target only for an Intel fallback runner.
if [ "$HOST_ARCH" = "arm64" ]; then
    PHASE=rust-ffi-release
    # SwiftPM owns the final arm64-apple-macosx deployment floor below.
    # Keep the native Rust host build free of MACOSX_DEPLOYMENT_TARGET: with
    # current macOS/Rust toolchains that setting can produce a proc-macro
    # dylib which rustc then rejects as an unavailable thiserror_impl crate.
    cargo build --locked --release -p folio-ffi
    FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/release/libfolio_ffi.a"
else
    PHASE=rust-ffi-cross-release
    rustup target add aarch64-apple-darwin
    CFLAGS_aarch64_apple_darwin="-mmacosx-version-min=$MACOS_TARGET_VERSION" \
    RUSTC_WRAPPER="$ROOT_DIR/packaging/rustc_macos_target_wrapper.sh" \
        cargo build --locked --target aarch64-apple-darwin --release -p folio-ffi
    FOLIOFORGE_FFI_ARCHIVE="$CARGO_TARGET_DIR/aarch64-apple-darwin/release/libfolio_ffi.a"
fi

PHASE=swift-release
SWIFT_TARGET_TRIPLE="arm64-apple-macosx$MACOS_TARGET_VERSION"
FOLIOFORGE_FFI_ARCHIVE="$FOLIOFORGE_FFI_ARCHIVE" \
    swift build --package-path macos/FolioForge --scratch-path "$SWIFT_BUILD_ROOT" \
        --triple "$SWIFT_TARGET_TRIPLE" -c release

PHASE=swift-product-path
PRODUCT_DIR=$(FOLIOFORGE_FFI_ARCHIVE="$FOLIOFORGE_FFI_ARCHIVE" \
    swift build --package-path macos/FolioForge --scratch-path "$SWIFT_BUILD_ROOT" \
        --triple "$SWIFT_TARGET_TRIPLE" -c release --show-bin-path)
DIST_DIR=${FOLIOFORGE_OUTPUT_ROOT:-"$ROOT_DIR/dist"}
PHASE=app-staging
BUILD_BASENAME="FolioForge-macOS-arm64-$MARKETING_VERSION-$BUILD_STAMP"
FINAL_DIR="$DIST_DIR/$BUILD_BASENAME"
FINAL_APP="$FINAL_DIR/FolioForge.app"
FINAL_ZIP="$DIST_DIR/$BUILD_BASENAME.zip"
STAGING=$(mktemp -d "$BUILD_ROOT/staging.XXXXXX")

if [ -e "$FINAL_DIR" ] || [ -e "$FINAL_ZIP" ]; then
    printf '%s\n' "Refusing to overwrite an existing build: $BUILD_BASENAME" >&2
    exit 2
fi

APP="$STAGING/FolioForge.app"
CONTENTS="$APP/Contents"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$PRODUCT_DIR/FolioForge" "$CONTENTS/MacOS/FolioForge"
PHASE=app-resource-staging
RESOURCE_BUNDLE_SOURCE="$PRODUCT_DIR/FolioForge_FolioForge.bundle"
RESOURCE_BUNDLE="$CONTENTS/Resources/FolioForge_FolioForge.bundle"
if [ -d "$RESOURCE_BUNDLE_SOURCE" ]; then
    mkdir -p "$RESOURCE_BUNDLE"
    cp -R "$RESOURCE_BUNDLE_SOURCE/Contents" "$RESOURCE_BUNDLE/"
else
    PHASE=app-resource-discovery
    RESOURCE_BUNDLE_SOURCE=$(find "$SWIFT_BUILD_ROOT" -type d \
        -name 'FolioForge_FolioForge.bundle' -print -quit)
    if [ -n "$RESOURCE_BUNDLE_SOURCE" ]; then
        PHASE=app-resource-copy-discovered
        mkdir -p "$RESOURCE_BUNDLE"
        cp -R "$RESOURCE_BUNDLE_SOURCE/Contents" "$RESOURCE_BUNDLE/"
    else
        # Some SwiftPM/Xcode combinations keep Bundle.module resources
        # inside the build intermediates instead of the product directory.
        # Recreate the standard resource bundle in external staging so the
        # unsigned app has the same runtime resource path on every runner.
        printf '%s\n' "SwiftPM resource bundle not emitted; staging checked-in resources" >&2
        PHASE=app-resource-fallback-directory
        mkdir -p "$RESOURCE_BUNDLE/Contents/Resources"
        PHASE=app-resource-fallback-plist
        cp packaging/FolioForge-Info.plist "$RESOURCE_BUNDLE/Contents/Info.plist"
        /usr/libexec/PlistBuddy -c 'Set :CFBundlePackageType BNDL' \
            "$RESOURCE_BUNDLE/Contents/Info.plist"
        /usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier org.folioforge.FolioForge.resources' \
            "$RESOURCE_BUNDLE/Contents/Info.plist"
        PHASE=app-resource-fallback-files
        cp -R "$ROOT_DIR/macos/FolioForge/Resources/." \
            "$RESOURCE_BUNDLE/Contents/Resources/"
    fi
fi
cp packaging/FolioForge-Info.plist "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $MARKETING_VERSION" \
    "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion $MACOS_TARGET_VERSION" \
    "$CONTENTS/Info.plist"

ICONSET="$STAGING/FolioForge.iconset"
PHASE=icon-generation
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

PHASE=app-validation
plutil -lint "$CONTENTS/Info.plist"
PHASE=app-version-check
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$CONTENTS/Info.plist")" = "$MARKETING_VERSION"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$CONTENTS/Info.plist")" = "$BUNDLE_BUILD_VERSION"
PHASE=app-deployment-version-check
test "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$CONTENTS/Info.plist")" = "$MACOS_TARGET_VERSION"
PHASE=app-content-executable
test -x "$CONTENTS/MacOS/FolioForge"
PHASE=app-signing
if [ "$SIGNING_MODE" = signed ]; then
    codesign --force --sign "$SIGNING_IDENTITY" --timestamp=none "$APP"
    codesign --verify --deep --strict "$APP"
else
    # The Apple linker may add an ad-hoc signature to a freshly linked
    # executable. Public GitHub artifacts are explicitly unsigned.
    codesign --remove-signature "$CONTENTS/MacOS/FolioForge" 2>/dev/null || true
    if codesign -dv "$CONTENTS/MacOS/FolioForge" >/dev/null 2>&1; then
        printf '%s\n' "Unexpected code signature in unsigned app executable" >&2
        exit 2
    fi
fi
PHASE=app-content-bundle-logo
test -f "$RESOURCE_BUNDLE/Contents/Resources/folioforge-logo.png"
PHASE=app-content-icon
test -f "$CONTENTS/Resources/FolioForge.icns"
PHASE=app-content-architecture
file "$CONTENTS/MacOS/FolioForge" | grep -q 'arm64'
PHASE=app-content-dependencies
if otool -L "$CONTENTS/MacOS/FolioForge" | sed '1d' | grep -Eq '/Volumes/Repositories|/opt/homebrew|/usr/local/opt|/target/'; then
    printf '%s\n' 'App executable contains a repository or private Homebrew dependency' >&2
    exit 2
fi
PHASE=app-content-library-boundary
if find "$APP" -type f \( -name 'library.sqlite' -o -name '*.sqlite' -o -name '*.sqlite3' \) -print -quit | grep -q .; then
    printf '%s\n' 'GUI bundle unexpectedly contains a Library database' >&2
    exit 2
fi

cp macos/FolioForge/README.md "$STAGING/README.md"
PHASE=zip-validation
ditto -c -k --sequesterRsrc --keepParent "$STAGING/FolioForge.app" \
    "$STAGING/FolioForge-macOS-arm64-$MARKETING_VERSION.zip"
unzip -t "$STAGING/FolioForge-macOS-arm64-$MARKETING_VERSION.zip"

# Publish to this unique build path only after validation passed.
PHASE=publish-artifact
mkdir -p "$FINAL_DIR"
cp -R "$STAGING/FolioForge.app" "$FINAL_DIR/FolioForge.app"
cp "$STAGING/README.md" "$FINAL_DIR/README.md"
cp "$STAGING/FolioForge-macOS-arm64-$MARKETING_VERSION.zip" "$FINAL_ZIP"

printf '%s\n' "Built and validated: $FINAL_APP" "$FINAL_ZIP"
printf '%s\n' "Code-signing mode: $SIGNING_MODE"
