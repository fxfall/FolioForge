#!/usr/bin/env python3
"""Persist content-free source-chain probes for the first C2-R candidates."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path
from typing import Any


SAMPLE_PATHS = {
    "KFX-C022": [("F002", "$146[0].$146[0]")],
    "KFX-C059": [
        ("F004", "$146[7].$146[0]"),
        ("F004", "$146[7].$146[2]"),
        ("F004", "$146[24].$146[1]"),
    ],
    "KFX-C075": [
        ("F026", "$146[4].$146[0]"),
        ("F026", "$146[4].$146[2]"),
    ],
    "KFX-C082": [("F046", "$146[0].$146[0].$146[0]")],
}
PRIVATE_KEYS = {"text", "private_text", "text_value", "symbol_name"}


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def command_version(command: list[str]) -> str:
    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        check=True,
        timeout=30,
    )
    return (result.stdout or result.stderr).strip()


def assert_content_free(value: Any) -> None:
    if isinstance(value, dict):
        forbidden = PRIVATE_KEYS.intersection(value)
        if forbidden:
            raise ValueError(f"kfx-dump returned private fields: {sorted(forbidden)}")
        for child in value.values():
            assert_content_free(child)
    elif isinstance(value, list):
        for child in value:
            assert_content_free(child)


def resolve_sources(book_root: Path, manifest: list[dict[str, Any]]) -> dict[str, Path]:
    wanted = {book["source_sha256_audit_only"] for book in manifest if book["id"] in SAMPLE_PATHS}
    found: dict[str, list[Path]] = {digest: [] for digest in wanted}
    for path in book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in found:
                found[digest].append(path)
    if any(len(paths) != 1 for paths in found.values()):
        raise ValueError("a minimum-source sample did not resolve uniquely by SHA-256")
    return {digest: paths[0] for digest, paths in found.items()}


def probe(
    book: dict[str, Any],
    source: Path,
    folio: Path,
    fragment: str,
    source_path: str,
) -> dict[str, Any]:
    result = subprocess.run(
        [
            str(folio),
            "kfx-dump",
            str(source),
            "--fragment",
            fragment,
            "--path",
            source_path,
            "--context",
        ],
        capture_output=True,
        text=True,
        check=False,
        timeout=120,
    )
    if result.returncode != 0:
        raise RuntimeError(f"content-free kfx-dump failed for {book['id']} {source_path}")
    dump = json.loads(result.stdout)
    assert_content_free(dump)
    if dump.get("input_sha256_audit_only") != book["source_sha256_audit_only"]:
        raise ValueError(f"kfx-dump source digest did not match {book['id']}")
    if dump.get("source_path") != source_path:
        raise ValueError(f"kfx-dump source path did not match {book['id']} {source_path}")
    if dump.get("fragment", {}).get("alias") != fragment:
        raise ValueError(f"kfx-dump fragment did not match {book['id']} {fragment}")
    fragment_summary = dump.get("fragment", {})
    if fragment_summary.get("relationship_basis") != "reading_order_to_section_to_story_reference":
        raise ValueError(f"reading-order relationship was not proven for {book['id']} {source_path}")
    if not fragment_summary.get("section_id") or not fragment_summary.get("story_id"):
        raise ValueError(f"section/story relationship was incomplete for {book['id']} {source_path}")
    return {
        "input_id": book["id"],
        "source_sha256_audit_only": book["source_sha256_audit_only"],
        "fragment": fragment,
        "source_path": source_path,
        "evidence": dump,
        "root_cause_status": "UnknownRootCause",
        "decision": "preserve_both_sides",
    }


def run(args: argparse.Namespace) -> int:
    manifest_doc = json.loads(args.manifest.read_text(encoding="utf-8"))
    manifest = manifest_doc.get("books")
    if not isinstance(manifest, list) or len(manifest) != 82:
        raise ValueError("pinned anonymous corpus must contain 82 books")
    by_id = {book["id"]: book for book in manifest}
    if set(SAMPLE_PATHS) - set(by_id):
        raise ValueError("minimum-source sample ID is missing from the corpus manifest")
    sources = resolve_sources(args.book_root, manifest)
    probes = []
    for book_id, paths in SAMPLE_PATHS.items():
        book = by_id[book_id]
        source = sources[book["source_sha256_audit_only"]]
        for fragment, source_path in paths:
            print(f"probing {book_id} {fragment} {source_path}", flush=True)
            probes.append(probe(book, source, args.folio, fragment, source_path))
    output = {
        "schema_version": 1,
        "scope": "content-free source-chain probes for C2-R minimum candidates",
        "private_text_included": False,
        "private_symbol_names_included": False,
        "toolchain": {
            "platform": platform.platform(),
            "folioforge_version": command_version([str(args.folio), "--version"]),
            "folioforge_binary_sha256_audit_only": file_sha256(args.folio),
        },
        "probe_count": len(probes),
        "probes": probes,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {len(probes)} content-free minimum-source probes", flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", type=Path, required=True)
    result.add_argument("--manifest", type=Path, default=Path("tests-private/kfx-corpus/corpus.json"))
    result.add_argument("--folio", type=Path, required=True)
    result.add_argument(
        "--output",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r/minimum-source-evidence.json"),
    )
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"C2 minimum-source evidence failed: {error}", file=sys.stderr)
        raise SystemExit(1)
