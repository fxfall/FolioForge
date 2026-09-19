#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
binary=${FOLIO_BIN:-"$repo/target/debug/folio"}
if [ ! -x "$binary" ]; then
    echo "folio binary not found at $binary; run cargo build -p folio-cli first" >&2
    exit 2
fi

source_epub=${1:-}
work=$(mktemp -d "${TMPDIR:-/tmp}/folioforge-matrix.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM
if [ -z "$source_epub" ]; then
    corpus="$work/corpus"
    "$repo/tests/scripts/build_minimal_corpus.sh" "$corpus" >/dev/null
    source_epub="$corpus/001-text.epub"
fi
case "$(printf '%s' "$source_epub" | tr '[:upper:]' '[:lower:]')" in
    *.epub|*.zip) ;;
    *) echo "matrix seed must be an EPUB path: $source_epub" >&2; exit 2 ;;
esac

seed_kf7="$work/seed.mobi"
seed_kf8="$work/seed.azw3"
seed_kfx="$work/seed.kfx"
"$binary" convert "$source_epub" --to kf7 --mode compatible --output "$seed_kf7" >"$work/seed-kf7.json" 2>"$work/seed-kf7.log"
"$binary" convert "$source_epub" --to kf8 --mode compatible --output "$seed_kf8" >"$work/seed-kf8.json" 2>"$work/seed-kf8.log"
"$binary" convert "$source_epub" --to kfx --mode compatible --output "$seed_kfx" >"$work/seed-kfx.json" 2>"$work/seed-kfx.log"

directions=0
cases=0
for source_format in epub kf7 kf8 kfx; do
    case "$source_format" in
        epub) input="$source_epub" ;;
        kf7) input="$seed_kf7" ;;
        kf8) input="$seed_kf8" ;;
        kfx) input="$seed_kfx" ;;
    esac
    for target in epub kf7 kf8 kfx; do
        [ "$source_format" = "$target" ] && continue
        directions=$((directions + 1))
        case "$target" in
            epub) extension=epub ;;
            kf7) extension=mobi ;;
            kf8) extension=azw3 ;;
            kfx) extension=kfx ;;
        esac
        for mode in strict compatible readable; do
            output="$work/${source_format}-to-${target}-${mode}.${extension}"
            report="$work/${source_format}-to-${target}-${mode}.json"
            log="$work/${source_format}-to-${target}-${mode}.log"
            if ! "$binary" convert "$input" --to "$target" --mode "$mode" --output "$output" >"$report" 2>"$log"; then
                echo "FAIL $source_format -> $target ($mode)" >&2
                sed -n '1,80p' "$log" >&2
                exit 1
            fi
            python3 - "$report" "$output" <<'PY'
import json
import os
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
assert os.path.isfile(sys.argv[2]), sys.argv[2]
assert report["round_trip"]["checked"] is True
assert report["round_trip"]["passed"] is True, report["round_trip"]
assert report["round_trip"]["unexpected_losses"] == []
PY
            if ! "$binary" validate "$output" >"$work/validate.json" 2>"$work/validate.log"; then
                echo "FAIL structural validation $source_format -> $target ($mode)" >&2
                sed -n '1,80p' "$work/validate.log" >&2
                exit 1
            fi
            cases=$((cases + 1))
            printf 'ok %s -> %s (%s)\n' "$source_format" "$target" "$mode"
        done
    done
done

printf 'conversion matrix directions=%s mode_cases=%s\n' "$directions" "$cases"
[ "$directions" -eq 12 ]
[ "$cases" -eq 36 ]
