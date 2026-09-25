#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT_DIR"

CORE_VERSION=$(awk '
    /^\[workspace\.package\]/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
' Cargo.toml)
VERSION=${FOLIOFORGE_RELEASE_VERSION:-$CORE_VERSION}
if [ -z "$CORE_VERSION" ] || [ "$VERSION" != "$CORE_VERSION" ]; then
    printf '%s\n' "Release version $VERSION must match Core version $CORE_VERSION" >&2
    exit 2
fi

HOST_ARCH=$(uname -m)
if [ "$HOST_ARCH" != "arm64" ]; then
    printf '%s\n' "The macOS Slint release requires a native Apple Silicon runner, found $HOST_ARCH" >&2
    exit 2
fi

VALIDATION_ROOT=${FOLIOFORGE_VALIDATION_ROOT:?set FOLIOFORGE_VALIDATION_ROOT to an external writable directory}
ARTIFACT_ROOT=${FOLIOFORGE_ARTIFACT_ROOT:?set FOLIOFORGE_ARTIFACT_ROOT to an external writable directory}
BUILD_ROOT=${FOLIOFORGE_SLINT_BUILD_ROOT:-$VALIDATION_ROOT/slint-macos-$VERSION}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$BUILD_ROOT/cargo-target}
TMPDIR=${TMPDIR:-$BUILD_ROOT/tmp}
FOLIOFORGE_TEMP_ROOT=${FOLIOFORGE_TEMP_ROOT:-$BUILD_ROOT/runtime}
export CARGO_TARGET_DIR TMPDIR FOLIOFORGE_TEMP_ROOT

mkdir -p "$CARGO_TARGET_DIR" "$TMPDIR" "$FOLIOFORGE_TEMP_ROOT" "$ARTIFACT_ROOT"

PHASE=build
cargo build --locked --release -p folioforge-slint
BINARY="$CARGO_TARGET_DIR/release/folioforge-slint"
test -x "$BINARY"
file "$BINARY" | grep -q 'arm64'

BUILD_STAMP=$(date '+%Y%m%d-%H%M%S')
STAGING=$(mktemp -d "$BUILD_ROOT/staging.XXXXXX")
cleanup() {
    exit_code=$?
    if [ -n "${STAGING:-}" ] && [ -d "$STAGING" ]; then
        rm -rf "$STAGING"
    fi
    if [ "$exit_code" -ne 0 ]; then
        printf '::error title=FolioForge Slint macOS packaging::phase=%s exit=%s\n' \
            "${PHASE:-unknown}" "$exit_code" >&2
    fi
    exit "$exit_code"
}
trap cleanup EXIT HUP INT TERM

PACKAGE_NAME="FolioForge-$VERSION-macos-arm64-slint"
PACKAGE_ROOT="$STAGING/$PACKAGE_NAME"
APP="$PACKAGE_ROOT/FolioForge-Slint.app"
CONTENTS="$APP/Contents"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"

PHASE=app-staging
cp "$BINARY" "$CONTENTS/MacOS/FolioForge-Slint"
cp packaging/FolioForge-Info.plist "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleExecutable FolioForge-Slint' "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.folioforge.app.slint' "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName FolioForge-Slint' "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName FolioForge-Slint' "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$CONTENTS/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :LSMinimumSystemVersion 27.0' "$CONTENTS/Info.plist"

ICONSET="$STAGING/FolioForge-Slint.iconset"
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

PHASE=unsigned-validation
codesign --remove-signature "$CONTENTS/MacOS/FolioForge-Slint" 2>/dev/null || true
if codesign -dv "$CONTENTS/MacOS/FolioForge-Slint" >/dev/null 2>&1; then
    printf '%s\n' 'Unexpected code signature in the unsigned Slint executable' >&2
    exit 2
fi
plutil -lint "$CONTENTS/Info.plist"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$CONTENTS/Info.plist")" = "$VERSION"
test "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$CONTENTS/Info.plist")" = '27.0'
test -f "$CONTENTS/Resources/FolioForge.icns"
if otool -L "$CONTENTS/MacOS/FolioForge-Slint" | sed '1d' | grep -Eq '/Volumes/Repositories|/opt/homebrew|/usr/local/opt|/target/'; then
    printf '%s\n' 'Slint app executable contains a repository or private Homebrew dependency' >&2
    exit 2
fi

cp LICENSE "$PACKAGE_ROOT/LICENSE"
cp packaging/RELEASE_PACKAGE_README.md "$PACKAGE_ROOT/README.md"
printf '%s\n' "$VERSION" > "$PACKAGE_ROOT/VERSION"

PHASE=archive
ARCHIVE="$ARTIFACT_ROOT/$PACKAGE_NAME.zip"
if [ -e "$ARCHIVE" ]; then
    printf '%s\n' "Refusing to overwrite existing release asset: $ARCHIVE" >&2
    exit 2
fi
ditto -c -k --sequesterRsrc --keepParent "$PACKAGE_ROOT" "$ARCHIVE"
unzip -t "$ARCHIVE"
printf '%s\n' "Built and validated unsigned Slint app: $ARCHIVE"
