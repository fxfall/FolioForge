#!/usr/bin/env python3
"""Run an isolated Calibre KFX Input oracle over anonymous hash-pinned inputs."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
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
        raise ValueError("anonymous oracle input manifest is empty or invalid")

    expected = {item["sha256"]: item for item in entries}
    if len(expected) != len(entries):
        raise ValueError("anonymous oracle manifest contains duplicate digests")
    matches: dict[str, list[Path]] = {digest: [] for digest in expected}
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

    calibre_version_output = subprocess.run(
        ["calibre-debug", "--version"], check=True, capture_output=True, text=True
    ).stdout.strip()
    calibre_version = calibre_version_output.rsplit(" ", 1)[-1]
    parser_script = Path(__file__).with_name("parse-json-content.py")
    args.output_dir.mkdir(parents=True, exist_ok=True)

    for index, item in enumerate(entries, start=1):
        raw_path = args.temp_dir / "raw-{:02d}.json".format(index)
        output_path = args.output_dir / (item["id"] + ".json")
        result = subprocess.run(
            [
                "calibre-debug",
                "-r",
                "KFX Input",
                "--",
                "--json-content",
                str(matches[item["sha256"]][0]),
                str(raw_path),
            ],
            env=os.environ.copy(),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if result.returncode != 0 or not raw_path.is_file():
            raise RuntimeError("{}: KFX Input oracle failed".format(item["id"]))
        parse = subprocess.run(
            [
                sys.executable,
                str(parser_script),
                "--input",
                str(raw_path),
                "--input-id",
                item["id"],
                "--input-sha256",
                item["sha256"],
                "--calibre-version",
                calibre_version,
                "--output",
                str(output_path),
            ],
            check=False,
        )
        raw_path.unlink(missing_ok=True)
        if parse.returncode != 0:
            raise RuntimeError("{}: content-free oracle normalization failed".format(item["id"]))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--temp-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        return run(args)
    except (OSError, KeyError, TypeError, ValueError, subprocess.CalledProcessError, RuntimeError):
        print("KFX placement oracle corpus run failed; no source metadata was emitted.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
