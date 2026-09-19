#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 5 ]]; then
  printf 'usage: %s BOOK_ROOT PRIVATE_OUTPUT_DIR [placement|text|text-diff|all-hunk|oracle-extra|minimum-source|three-way|boko-crosscheck|all] [ANONYMOUS_ID_OR_BOKO_PATH] [ANONYMOUS_ID]\n' "$0" >&2
  exit 2
fi

book_root="$1"
output_dir="$2"
audit_mode="${3:-placement}"
target_book_id="${4:-}"
boko_book_id="${5:-}"
if [[ "$audit_mode" != "placement" && "$audit_mode" != "text" && "$audit_mode" != "text-diff" && "$audit_mode" != "all-hunk" && "$audit_mode" != "oracle-extra" && "$audit_mode" != "minimum-source" && "$audit_mode" != "three-way" && "$audit_mode" != "boko-crosscheck" && "$audit_mode" != "all" ]]; then
  printf 'audit mode must be placement, text, text-diff, all-hunk, oracle-extra, minimum-source, three-way, boko-crosscheck, or all.\n' >&2
  exit 2
fi
if [[ -n "$target_book_id" && "$audit_mode" != "text-diff" && "$audit_mode" != "boko-crosscheck" && "$audit_mode" != "all-hunk" && "$audit_mode" != "oracle-extra" && "$audit_mode" != "three-way" ]]; then
  printf 'The optional fourth argument is supported only with text-diff, all-hunk, oracle-extra, three-way, or boko-crosscheck mode.\n' >&2
  exit 2
fi
if [[ -n "$boko_book_id" && "$audit_mode" != "boko-crosscheck" && "$audit_mode" != "oracle-extra" && "$audit_mode" != "all-hunk" && "$audit_mode" != "three-way" ]]; then
  printf 'The optional fifth argument is supported only with boko-crosscheck, oracle-extra, all-hunk, or three-way mode.\n' >&2
  exit 2
fi
if [[ "$audit_mode" == "boko-crosscheck" && -z "$target_book_id" ]]; then
  printf 'boko-crosscheck mode requires the external Bokō executable path.\n' >&2
  exit 2
fi
script_dir="$(cd "$(dirname "$0")" && pwd)"
project_root="$(cd "$script_dir/../../.." && pwd)"
corpus_manifest="$project_root/tests-private/kfx-corpus/corpus.json"

if [[ ! -d "$book_root" || ! -f "$corpus_manifest" ]]; then
  printf 'C1 corpus input or private FolioForge manifest is unavailable.\n' >&2
  exit 2
fi
if ! command -v calibre-debug >/dev/null 2>&1 || ! command -v calibre-customize >/dev/null 2>&1; then
  printf 'Calibre 9.14 or compatible is required.\n' >&2
  exit 2
fi

temp_dir="$(mktemp -d)"
cleanup() {
  python3 -c 'import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)' "$temp_dir"
}
trap cleanup EXIT INT TERM
export CALIBRE_CONFIG_DIRECTORY="$temp_dir/calibre-config"

plugin_url="https://plugins.calibre-ebook.com/291290.zip"
plugin_sha256="338809c18e5f9bb721dc3570a64cf6f7add4a1711c8183e6179e90fcbadf3c1d"
curl --fail --silent --show-error --location --connect-timeout 15 --max-time 90 \
  "$plugin_url" --output "$temp_dir/kfx-input.zip"
actual_sha256="$(shasum -a 256 "$temp_dir/kfx-input.zip" | awk '{print $1}')"
if [[ "$actual_sha256" != "$plugin_sha256" ]]; then
  printf 'KFX Input plugin archive checksum changed; refusing an unpinned oracle.\n' >&2
  exit 1
fi

python3 - "$corpus_manifest" "$temp_dir/inputs.json" <<'PY'
import json
import sys
from pathlib import Path

source = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
books = source.get("books")
if not isinstance(books, list) or len(books) != 82:
    raise SystemExit("C1 manifest does not contain the expected 82 anonymous books")
inputs = [
    {"id": book["id"], "sha256": book["source_sha256_audit_only"]}
    for book in books
]
if len({item["sha256"] for item in inputs}) != len(inputs):
    raise SystemExit("C1 manifest contains duplicate source hashes")
Path(sys.argv[2]).write_text(
    json.dumps({"inputs": inputs}, ensure_ascii=False, indent=2) + "\n",
    encoding="utf-8",
)
PY

calibre-customize --add-plugin "$temp_dir/kfx-input.zip" >/dev/null 2>&1
calibre-debug -r "KFX Input" -- --help >/dev/null 2>&1
mkdir -p "$temp_dir/work" "$output_dir"
if [[ "$audit_mode" == "placement" || "$audit_mode" == "all" ]]; then
  python3 "$script_dir/compare-calibre-identities.py" \
    --book-root "$book_root" \
    --manifest "$temp_dir/inputs.json" \
    --folio "$project_root/target/debug/folio" \
    --temp-dir "$temp_dir/work" \
    --output "$output_dir/calibre-identity-comparison.json"
  printf 'Calibre placement comparison completed for 82 anonymous inputs.\n'
fi
if [[ "$audit_mode" == "text" || "$audit_mode" == "all" ]]; then
  python3 "$script_dir/compare-calibre-text.py" \
    --book-root "$book_root" \
    --manifest "$temp_dir/inputs.json" \
    --fidelity-report-dir "$project_root/tests-private/kfx-corpus/reports" \
    --folio "$project_root/target/debug/folio" \
    --temp-dir "$temp_dir/work" \
    --output "$output_dir/calibre-text-comparison.json"
  printf 'Calibre/native/semantic/IR/EPUB text comparison completed for 82 anonymous inputs.\n'
fi
if [[ "$audit_mode" == "text-diff" || "$audit_mode" == "all" ]]; then
  diff_args=(
    --book-root "$book_root"
    --manifest "$corpus_manifest"
    --comparison "$project_root/tests-private/kfx-corpus/calibre-c2/calibre-text-comparison.json"
    --fidelity-report-dir "$project_root/tests-private/kfx-corpus/reports"
    --folio "$project_root/target/debug/folio"
    --temp-dir "$temp_dir/work"
    --output "$output_dir"
  )
  if [[ -n "$target_book_id" ]]; then
    diff_args+=(--only "$target_book_id")
  fi
  python3 "$script_dir/run-c2-text-diff.py" "${diff_args[@]}"
  printf 'C2-R content-free Unicode hunk reports generated.\n'
fi
if [[ "$audit_mode" == "all-hunk" ]]; then
  all_hunk_args=(
    --book-root "$book_root"
    --manifest "$corpus_manifest"
    --report-dir "$project_root/tests-private/kfx-corpus/c2-r"
    --folio "$project_root/target/debug/folio"
    --output "$output_dir/all-hunk-audit.json"
  )
  if [[ -n "$target_book_id" ]]; then
    all_hunk_args+=(--only "$target_book_id")
  fi
  if [[ -n "$boko_book_id" ]]; then
    all_hunk_args+=(--only "$boko_book_id")
  fi
  python3 "$script_dir/run-c2-all-hunk-audit.py" "${all_hunk_args[@]}"
  printf 'C2-R all-direction hunk audit generated.\n'
fi
if [[ "$audit_mode" == "oracle-extra" ]]; then
  oracle_extra_args=(
    --book-root "$book_root"
    --report-dir "$project_root/tests-private/kfx-corpus/c2-r"
    --folio "$project_root/target/debug/folio"
    --output "$output_dir/oracle-extra-inventory-v2.json"
  )
  if [[ -n "$target_book_id" ]]; then
    oracle_extra_args+=(--only "$target_book_id")
  fi
  if [[ -n "$boko_book_id" ]]; then
    oracle_extra_args+=(--only "$boko_book_id")
  fi
  python3 "$script_dir/run-c2-oracle-extra-inventory.py" "${oracle_extra_args[@]}"
  printf 'C2-R event-split Oracle-extra source audit generated.\n'
fi
if [[ "$audit_mode" == "minimum-source" ]]; then
  python3 "$script_dir/run-c2-minimum-source-evidence.py" \
    --book-root "$book_root" \
    --manifest "$corpus_manifest" \
    --folio "$project_root/target/debug/folio" \
    --output "$output_dir/minimum-source-evidence.json"
  printf 'C2-R minimum source-chain evidence generated.\n'
fi
if [[ "$audit_mode" == "three-way" ]]; then
  if [[ -z "$target_book_id" ]]; then
    printf 'three-way mode requires the external Bokō executable path.\n' >&2
    exit 2
  fi
  three_way_args=(
    --book-root "$book_root"
    --manifest "$corpus_manifest"
    --comparison "$project_root/tests-private/kfx-corpus/calibre-c2/calibre-text-comparison.json"
    --folio "$project_root/target/debug/folio"
    --boko "$target_book_id"
    --output "$output_dir/three-way-output-comparison.json"
  )
  if [[ -n "$boko_book_id" ]]; then
    three_way_args+=(--only "$boko_book_id")
  fi
  python3 "$script_dir/run-c2-three-way-output-comparison.py" "${three_way_args[@]}"
  printf 'Calibre/Bokō/FolioForge EPUB output comparison completed.\n'
fi
if [[ "$audit_mode" == "boko-crosscheck" ]]; then
  boko_args=(
    --book-root "$book_root" \
    --manifest "$corpus_manifest" \
    --comparison "$project_root/tests-private/kfx-corpus/calibre-c2/calibre-text-comparison.json" \
    --c2r-dir "$output_dir" \
    --folio "$project_root/target/debug/folio" \
    --boko "$target_book_id" \
    --temp-dir "$temp_dir/work" \
    --output "$output_dir/boko-comparison.json"
  )
  if [[ -n "$boko_book_id" ]]; then
    boko_args+=(--only "$boko_book_id")
  fi
  python3 "$script_dir/run-c2-boko-crosscheck.py" "${boko_args[@]}"
  printf 'Calibre/Bokō/Folio EPUB text cross-check completed.\n'
fi
