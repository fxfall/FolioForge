#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
validation_root=${FOLIOFORGE_PHASE7_VALIDATION:-${FOLIOFORGE_VALIDATION_ROOT:?set FOLIOFORGE_VALIDATION_ROOT}}
target_dir="$validation_root/benchmark-target"
mkdir -p "$validation_root/tmp" "$validation_root/runtime" "$target_dir"

CARGO_TARGET_DIR="$target_dir" TMPDIR="$validation_root/tmp" \
  FOLIOFORGE_TEMP_ROOT="$validation_root/runtime" \
  cargo build --release --manifest-path "$repo_root/Cargo.toml" -p folio-cli

python3 "$repo_root/tools/release/run-benchmarks.py" \
  --binary "$target_dir/release/folio" \
  --output-root "$validation_root/benchmark" \
  "$@"
