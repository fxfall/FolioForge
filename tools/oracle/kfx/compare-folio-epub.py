#!/usr/bin/env python3
"""Verify image-reference preservation through FolioForge EPUB serialization."""

from __future__ import annotations

import argparse
import hashlib
import json
import posixpath
import subprocess
import urllib.parse
import zipfile
from collections import Counter
from pathlib import Path
from typing import Any
from xml.etree import ElementTree


OPF_NS = "http://www.idpf.org/2007/opf"
CONTAINER_NS = "urn:oasis:names:tc:opendocument:xmlns:container"
XLINK_HREF = "{http://www.w3.org/1999/xlink}href"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def href_path(document_path: str, href: str) -> str | None:
    parsed = urllib.parse.urlsplit(href)
    if parsed.scheme or parsed.netloc or not parsed.path:
        return None
    decoded_path = urllib.parse.unquote(parsed.path)
    return posixpath.normpath(posixpath.join(posixpath.dirname(document_path), decoded_path))


def epub_image_references(
    epub_path: Path, audit: dict[str, Any]
) -> tuple[Counter[str], int, int, list[str | None]]:
    with zipfile.ZipFile(epub_path) as archive:
        names = set(archive.namelist())
        container = ElementTree.fromstring(archive.read("META-INF/container.xml"))
        rootfile = container.find(".//{%s}rootfile" % CONTAINER_NS)
        if rootfile is None or not rootfile.get("full-path"):
            raise ValueError("EPUB package rootfile is missing")
        package_path = rootfile.get("full-path")
        package_dir = posixpath.dirname(package_path)
        package = ElementTree.fromstring(archive.read(package_path))
        manifest = package.find("{%s}manifest" % OPF_NS)
        if manifest is None:
            raise ValueError("EPUB manifest is missing")
        manifest_ids: dict[str, str] = {}
        paths_by_manifest_id: dict[str, str] = {}
        for item in manifest.findall("{%s}item" % OPF_NS):
            item_id = item.get("id")
            href = item.get("href")
            media_type = item.get("media-type")
            if not item_id or not href:
                continue
            path = posixpath.normpath(posixpath.join(package_dir, urllib.parse.unquote(href)))
            manifest_ids[path] = item_id
            paths_by_manifest_id[item_id] = path

        spine = package.find("{%s}spine" % OPF_NS)
        if spine is None:
            raise ValueError("EPUB spine is missing")
        document_paths: list[str] = []
        for itemref in spine.findall("{%s}itemref" % OPF_NS):
            item_id = itemref.get("idref")
            path = paths_by_manifest_id.get(item_id or "")
            if path is not None and path in names:
                document_paths.append(path)

        aliases_by_manifest = {
            item["manifest_id"]: item["resource"]
            for item in audit.get("epub_resource_map", [])
        }
        frequencies: Counter[str] = Counter()
        total_image_references = 0
        unmapped_references = 0
        sequence: list[str | None] = []
        for document_path in document_paths:
            document = ElementTree.fromstring(archive.read(document_path))
            for element in document.iter():
                tag = element.tag.rsplit("}", 1)[-1].lower() if isinstance(element.tag, str) else ""
                if tag == "img":
                    href = element.get("src")
                elif tag == "image":
                    href = element.get("href") or element.get(XLINK_HREF)
                else:
                    continue
                if not href:
                    continue
                total_image_references += 1
                resource_path = href_path(document_path, href)
                manifest_id = manifest_ids.get(resource_path or "")
                resource_alias = aliases_by_manifest.get(manifest_id or "")
                sequence.append(resource_alias)
                if resource_alias is None:
                    unmapped_references += 1
                else:
                    frequencies[resource_alias] += 1
        return frequencies, total_image_references, unmapped_references, sequence


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
        if len(matches[item["sha256"]]) != 1:
            raise ValueError("pinned local input did not resolve uniquely")

    results: list[dict[str, Any]] = []
    for item in entries:
        audit_path = args.audit_dir / (item["id"] + ".json")
        oracle_path = args.oracle_dir / (item["id"] + ".json")
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
        oracle = json.loads(oracle_path.read_text(encoding="utf-8"))
        epub_path = args.temp_dir / (item["id"] + ".epub")
        try:
            subprocess.run(
                [
                    str(args.folio),
                    "convert",
                    str(matches[item["sha256"]][0]),
                    "--to",
                    "epub",
                    "--output",
                    str(epub_path),
                ],
                check=True,
                capture_output=True,
                text=True,
            )
            frequencies, total, unmapped, epub_sequence = epub_image_references(epub_path, audit)
            ir_frequencies = Counter(
                {
                    row["resource"]: row["count"]
                    for row in audit["ir_image_nodes"]["resource_frequencies"]
                }
            )
            oracle_frequencies = Counter(
                {row["resource"]: row["count"] for row in oracle["resource_frequencies"]}
            )
            ir_sequence = [
                row["resource"] for row in audit["ir_image_nodes"]["occurrences"]
            ]
            oracle_sequence = [row["resource"] for row in oracle["image_occurrences"]]
            epub_validation = subprocess.run(
                [str(args.folio), "validate", str(epub_path)],
                check=False,
                capture_output=True,
                text=True,
            )
            results.append(
                {
                    "input_id": item["id"],
                    "oracle_image_references": len(oracle_sequence),
                    "ir_image_nodes": len(ir_sequence),
                    "epub_image_references": total,
                    "epub_unmapped_image_references": unmapped,
                    "epub_resource_frequencies_equal_ir": frequencies == ir_frequencies,
                    "ir_resource_frequencies_equal_oracle": ir_frequencies == oracle_frequencies,
                    "ir_sequence_equal_oracle": ir_sequence == oracle_sequence,
                    "epub_sequence_equal_ir": epub_sequence == ir_sequence,
                    "epub_sequence_equal_oracle": epub_sequence == oracle_sequence,
                    "epub_validation_passed": epub_validation.returncode == 0,
                }
            )
        finally:
            epub_path.unlink(missing_ok=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "comparison": "anonymous-folio-ir-to-epub-image-reference-parity",
                "books": results,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--audit-dir", required=True, type=Path)
    parser.add_argument("--oracle-dir", required=True, type=Path)
    parser.add_argument("--temp-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        return run(args)
    except (
        OSError,
        KeyError,
        TypeError,
        ValueError,
        zipfile.BadZipFile,
        ElementTree.ParseError,
        subprocess.CalledProcessError,
        json.JSONDecodeError,
    ) as error:
        parser.error(
            "EPUB comparison failed ({}); source metadata and paths were not emitted".format(
                type(error).__name__
            )
        )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
