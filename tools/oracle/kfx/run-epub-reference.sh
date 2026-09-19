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

temp_dir="$(mktemp -d)"
cleanup() {
  python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1])' "$temp_dir"
}
trap cleanup EXIT INT TERM

python3 "$script_dir/compare-folio-epub.py" \
  --book-root "$book_root" \
  --manifest "$project_root/tests/kfx-placement-oracle/inputs.json" \
  --folio "$project_root/target/debug/folio" \
  --audit-dir "$project_root/tests/kfx-placement-oracle/reports" \
  --oracle-dir "$project_root/tests/kfx-placement-oracle/expected" \
  --temp-dir "$temp_dir" \
  --output "$output_file"
