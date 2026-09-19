#!/usr/bin/env python3
"""Reduce Calibre KFX Input --json-content output to content-free image data."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any


def parse_oracle(raw: Any, input_id: str, input_sha256: str, calibre_version: str) -> dict[str, Any]:
    if not isinstance(raw, dict) or not isinstance(raw.get("data"), list):
        raise ValueError("unexpected KFX Input JSON structure")

    image_rows: list[tuple[int, int, str]] = []
    for ordinal, item in enumerate(raw["data"]):
        if not isinstance(item, dict) or item.get("type") != 2:
            continue
        resource_name = item.get("content")
        position = item.get("position")
        if not isinstance(resource_name, str) or not isinstance(position, int) or position < 0:
            raise ValueError("invalid KFX image-position record")
        image_rows.append((position, ordinal, resource_name))

    # Sorting resource keys before assigning aliases makes aliases independent
    # of JSON traversal order while ensuring no source resource names escape.
    resource_names = sorted({row[2] for row in image_rows})
    aliases = {
        name: "img_{:04d}".format(index)
        for index, name in enumerate(resource_names, start=1)
    }
    ordered = sorted(image_rows, key=lambda row: (row[0], row[1]))
    frequencies = Counter(row[2] for row in image_rows)

    return {
        "schema_version": 1,
        "oracle": "calibre-kfx-input-json-content",
        "calibre_version": calibre_version,
        "kfx_input_version": "2.34.2",
        "input_id": input_id,
        "input_sha256": input_sha256,
        "image_occurrences": [
            {"position": position, "resource": aliases[name]}
            for position, _, name in ordered
        ],
        "resource_frequencies": [
            {"resource": aliases[name], "count": frequencies[name]}
            for name in resource_names
        ],
        "summary": {
            "image_occurrences": len(image_rows),
            "distinct_resources": len(resource_names),
            "reused_resources": sum(1 for count in frequencies.values() if count > 1),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--input-id", required=True)
    parser.add_argument("--input-sha256", required=True)
    parser.add_argument("--calibre-version", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    if len(args.input_sha256) != 64 or any(c not in "0123456789abcdef" for c in args.input_sha256):
        parser.error("--input-sha256 must be a lowercase SHA-256 digest")
    try:
        raw = json.loads(args.input.read_text(encoding="utf-8"))
        sanitized = parse_oracle(raw, args.input_id, args.input_sha256, args.calibre_version)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError):
        parser.error("could not parse a valid KFX Input JSON-content file")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(sanitized, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        "{}: {} image occurrences across {} resources".format(
            args.input_id,
            sanitized["summary"]["image_occurrences"],
            sanitized["summary"]["distinct_resources"],
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
