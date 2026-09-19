#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../corpus/minimal/basic" && pwd)
output=${1:-"/tmp/folioforge-basic.epub"}
work=$(mktemp -d "${TMPDIR:-/tmp}/folioforge-epub.XXXXXX")
trap 'rm -rf "$work"' EXIT INT TERM

mkdir -p "$work/META-INF" "$work/OEBPS"
# OCF requires the mimetype entry to contain exactly this value, without a
# trailing newline, and to be the first stored ZIP entry.
printf '%s' 'application/epub+zip' > "$work/mimetype"
cp -R "$root/META-INF/." "$work/META-INF/"
cp -R "$root/OEBPS/." "$work/OEBPS/"
mkdir -p "$(dirname -- "$output")"
rm -f "$output"
(cd "$work" && zip -X -q -0 "$output" mimetype && zip -X -q -r "$output" META-INF OEBPS)
printf '%s\n' "$output"
