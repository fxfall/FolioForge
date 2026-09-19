#!/bin/sh
set -eu

RUSTC=$1
shift
TARGET=""
EXPECT_TARGET=0

for ARG in "$@"; do
    if [ "$EXPECT_TARGET" -eq 1 ]; then
        TARGET=$ARG
        EXPECT_TARGET=0
        continue
    fi
    case "$ARG" in
        --target)
            EXPECT_TARGET=1
            ;;
        --target=*)
            TARGET=${ARG#--target=}
            ;;
    esac
done

# Keep host procedural macros at the host default. Only Rust units explicitly
# compiled for the macOS app target receive the deployment-version override.
if [ "$TARGET" = "aarch64-apple-darwin" ]; then
    MACOSX_DEPLOYMENT_TARGET=13.0 exec "$RUSTC" "$@"
fi

exec "$RUSTC" "$@"
