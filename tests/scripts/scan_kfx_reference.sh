#!/bin/sh
set -eu

corpus=${1:-${FOLIOFORGE_KFX_CORPUS:-private-corpus}}
binary=${FOLIO_BIN:-target/debug/folio}

if [ ! -x "$binary" ]; then
    echo "folio binary not found at $binary; run cargo build -p folio-cli first" >&2
    exit 2
fi

if [ ! -d "$corpus" ]; then
    echo "KFX corpus directory not found: $corpus" >&2
    exit 2
fi

scan_results=$(find "$corpus" -type f -iname '*.kfx' -exec sh -c '
    binary=$1
    shift
    for input do
        if "$binary" inspect "$input" --ion >/dev/null 2>&1; then
            printf .
        else
            printf !
        fi
    done
' sh "$binary" {} +)

scanned=$(printf '%s' "$scan_results" | wc -c | tr -d '[:space:]')
failures=$(printf '%s' "$scan_results" | tr -cd '!' | wc -c | tr -d '[:space:]')
if [ "$scanned" -eq 0 ]; then
    echo "no .kfx files found under $corpus" >&2
    exit 2
fi

printf 'scanned=%s failures=%s corpus=%s\n' "$scanned" "$failures" "$corpus"
[ "$failures" -eq 0 ]
