#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  printf 'usage: %s BOOK_ROOT OUTPUT_JSON\n' "$0" >&2
  exit 2
fi

book_root="$1"
output_file="$2"
script_dir="$(cd "$(dirname "$0")" && pwd)"
project_root="$(cd "$script_dir/../../.." && pwd)"

if [[ ! -d "$book_root" ]]; then
  printf 'book root is not a directory\n' >&2
  exit 2
fi
if ! command -v calibre-debug >/dev/null 2>&1 || ! command -v calibre-customize >/dev/null 2>&1; then
  printf 'Calibre 9.14 or compatible is required.\n' >&2
  exit 2
fi

temp_dir="$(mktemp -d)"
cleanup() {
  python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1])' "$temp_dir"
}
trap cleanup EXIT INT TERM
export CALIBRE_CONFIG_DIRECTORY="$temp_dir/calibre-config"

plugin_url="https://plugins.calibre-ebook.com/291290.zip"
plugin_sha256="338809c18e5f9bb721dc3570a64cf6f7add4a1711c8183e6179e90fcbadf3c1d"
curl --fail --silent --show-error --location "$plugin_url" --output "$temp_dir/kfx-input.zip"
actual_sha256="$(shasum -a 256 "$temp_dir/kfx-input.zip" | awk '{print $1}')"
if [[ "$actual_sha256" != "$plugin_sha256" ]]; then
  printf 'KFX Input plugin archive checksum changed; refusing an unpinned oracle.\n' >&2
  exit 1
fi
calibre-customize --add-plugin "$temp_dir/kfx-input.zip" >/dev/null 2>&1
calibre-debug -r "KFX Input" -- --help >/dev/null 2>&1

python3 "$script_dir/compare-calibre-identities.py" \
  --book-root "$book_root" \
  --manifest "$project_root/tests/kfx-placement-oracle/inputs.json" \
  --folio "$project_root/target/debug/folio" \
  --temp-dir "$temp_dir" \
  --output "$output_file"
