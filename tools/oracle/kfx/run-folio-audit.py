#!/usr/bin/env python3
"""Collect anonymous FolioForge placement traces for hash-pinned KFX inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    entries = manifest.get("inputs")
    if not isinstance(entries, list) or not entries:
        raise ValueError("anonymous placement input manifest is empty or invalid")

    matches: dict[str, list[Path]] = {item["sha256"]: [] for item in entries}
    for path in args.book_root.rglob("*"):
        if path.is_file():
            digest = file_sha256(path)
            if digest in matches:
                matches[digest].append(path)
    for item in entries:
        found = matches[item["sha256"]]
        if len(found) != 1:
            raise ValueError(
                "{}: expected one local input matching the pinned digest, found {}".format(
                    item["id"], len(found)
                )
            )

    args.output_dir.mkdir(parents=True, exist_ok=True)
    for item in entries:
        result = subprocess.run(
            [str(args.folio), "inspect", str(matches[item["sha256"]][0]), "--kfx-placement-audit"],
            check=True,
            capture_output=True,
            text=True,
        )
        report = json.loads(result.stdout)
        if report.get("input_sha256_audit_only") != item["sha256"]:
            raise ValueError("{}: FolioForge audit digest did not match the pinned input".format(item["id"]))
        destination = args.output_dir / (item["id"] + ".json")
        destination.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, KeyError, TypeError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError):
        parser.error("placement audit corpus run failed; no source metadata was emitted")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
