#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo"
cargo build --locked -p folio-cli -p folio-service -p folio-ffi
exec python3 "$repo/tests/parity/run.py"
