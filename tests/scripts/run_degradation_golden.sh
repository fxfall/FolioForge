#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
binary=${FOLIO_BIN:-"$repo/target/debug/folio"}
manifest="$repo/tests/degradation/manifest.tsv"
if [ ! -x "$binary" ]; then
    echo "folio binary not found at $binary; run cargo build -p folio-cli first" >&2
    exit 2
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/folioforge-degradation.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM
passed=0

while IFS="$(printf '\t')" read -r fixture target mode feature quality fallback; do
    [ -n "$fixture" ] || continue
    case "$fixture" in \#*) continue ;; esac

    source_dir="$repo/tests/degradation/$fixture"
    package="$work/$fixture"
    mkdir -p "$package/META-INF" "$package/OEBPS"
    printf '%s' 'application/epub+zip' > "$package/mimetype"
    cat > "$package/META-INF/container.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>
EOF
    cp "$source_dir/source.xhtml" "$package/OEBPS/chapter.xhtml"

    style_item=
    if [ -f "$source_dir/style.css" ]; then
        cp "$source_dir/style.css" "$package/OEBPS/style.css"
        style_item='<item id="style" href="style.css" media-type="text/css"/>'
    fi
    image_item=
    if [ -f "$source_dir/image.svg" ]; then
        cp "$source_dir/image.svg" "$package/OEBPS/image.svg"
        image_item='<item id="image" href="image.svg" media-type="image/svg+xml"/>'
    fi
    font_item=
    if [ -f "$source_dir/font.woff" ]; then
        cp "$source_dir/font.woff" "$package/OEBPS/font.woff"
        font_item='<item id="font" href="font.woff" media-type="font/woff"/>'
    fi
    fixed_meta=
    if [ "$fixture" = fixed-layout ]; then
        fixed_meta='<meta property="rendition:layout">pre-paginated</meta>'
    fi
    cat > "$package/OEBPS/content.opf" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:folioforge:golden:$fixture</dc:identifier><dc:title>$fixture</dc:title><dc:language>en</dc:language>$fixed_meta</metadata>
  <manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>$style_item$image_item$font_item</manifest>
  <spine><itemref idref="chapter"/></spine>
</package>
EOF
    epub="$work/$fixture.epub"
    (cd "$package" && zip -X -q -0 "$epub" mimetype && zip -X -q -r "$epub" META-INF OEBPS)
    plan="$work/$fixture.json"
    if ! "$binary" analyze "$epub" --to "$target" --mode "$mode" > "$plan"; then
        echo "FAIL $fixture: analyze command failed" >&2
        exit 1
    fi
    strict_plan="$work/$fixture-strict.json"
    readable_plan="$work/$fixture-readable.json"
    "$binary" analyze "$epub" --to "$target" --mode strict > "$strict_plan"
    "$binary" analyze "$epub" --to "$target" --mode readable > "$readable_plan"
    python3 - "$plan" "$source_dir/expected.json" "$feature" "$quality" "$fallback" <<'PY'
import json
import sys

plan = json.load(open(sys.argv[1], encoding="utf-8"))
expected = json.load(open(sys.argv[2], encoding="utf-8"))
feature, quality, fallback = sys.argv[3:]
assert plan["target"] == expected["target"], (plan["target"], expected["target"])
assert plan["mode"] == expected["mode"], (plan["mode"], expected["mode"])
matches = [item for item in plan["items"] if item["feature"] == feature]
assert matches, (feature, plan["items"])
item = matches[0]
assert item["quality"] == quality, (feature, item["quality"], quality)
assert fallback in item["selected_fallback"], (feature, item["selected_fallback"], fallback)
if quality != "Exact":
    assert item["diagnostic"]["severity"] in ("Warning", "Error")
    assert item in plan["items"]
assert plan["blocked"] is False
PY
    python3 - "$strict_plan" "$readable_plan" "$quality" <<'PY'
import json
import sys

strict = json.load(open(sys.argv[1], encoding="utf-8"))
readable = json.load(open(sys.argv[2], encoding="utf-8"))
quality = sys.argv[3]
assert strict["blocked"] is (quality not in ("Exact", "Equivalent")), (quality, strict["blocked"])
assert readable["blocked"] is False
PY
    passed=$((passed + 1))
    printf 'ok %s (%s/%s)\n' "$fixture" "$quality" "$feature"
done < "$manifest"

printf 'degradation golden cases passed=%s\n' "$passed"
