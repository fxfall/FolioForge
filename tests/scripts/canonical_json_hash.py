#!/usr/bin/env python3
"""Hash a JSON document after deterministic semantic canonicalization."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: canonical_json_hash.py INPUT_JSON OUTPUT_HASH",
            file=sys.stderr,
        )
        return 2

    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])
    value = json.loads(input_path.read_text(encoding="utf-8"))
    canonical = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    digest = hashlib.sha256(canonical).hexdigest()
    output_path.write_text(f"{digest}  {input_path.name}\n", encoding="utf-8")
    print(f"{digest}  {input_path.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
