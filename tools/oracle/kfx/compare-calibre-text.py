#!/usr/bin/env python3
"""Compare Calibre's ordered visible KFX text with anonymous Folio IR audits."""

from __future__ import annotations

import argparse
import hashlib
import json
import posixpath
import subprocess
import unicodedata
import urllib.parse
import zipfile
from pathlib import Path
from typing import Any
from xml.etree import ElementTree


CONTAINER_NS = "urn:oasis:names:tc:opendocument:xmlns:container"
OPF_NS = "http://www.idpf.org/2007/opf"
XHTML_NS = "http://www.w3.org/1999/xhtml"


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ordered_calibre_text(raw: dict[str, Any]) -> tuple[str, int]:
    rows = raw.get("data")
    if not isinstance(rows, list):
        raise ValueError("unexpected KFX Input JSON structure")
    occurrences = [
        (row["position"], ordinal, row["content"])
        for ordinal, row in enumerate(rows)
        if isinstance(row, dict)
        and row.get("type") == 1
        and isinstance(row.get("position"), int)
        and isinstance(row.get("content"), str)
    ]
    occurrences.sort(key=lambda occurrence: (occurrence[0], occurrence[1]))
    return "".join(occurrence[2] for occurrence in occurrences), len(occurrences)


def summarize_calibre_text(raw: dict[str, Any]) -> dict[str, Any]:
    visible_text, fragment_count = ordered_calibre_text(raw)
    summary = summarize_text(visible_text)
    summary["text_fragment_count"] = fragment_count
    return summary


def summarize_text(visible_text: str) -> dict[str, Any]:
    normalized = unicodedata.normalize(
        "NFC", visible_text.replace("\r\n", "\n").replace("\r", "\n")
    )
    return {
        "unicode_scalar_count": len(normalized),
        "normalized_sha256_audit_only": hashlib.sha256(
            normalized.encode("utf-8")
        ).hexdigest(),
    }


def epub_visible_text_stream(epub_path: Path) -> tuple[str, int, int]:
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
        spine = package.find("{%s}spine" % OPF_NS)
        if manifest is None or spine is None:
            raise ValueError("EPUB manifest or spine is missing")
        hrefs = {
            item.get("id"): item.get("href")
            for item in manifest.findall("{%s}item" % OPF_NS)
            if item.get("id") and item.get("href")
        }
        visible_pieces: list[str] = []
        xml_text_node_count = 0
        document_count = 0
        for itemref in spine.findall("{%s}itemref" % OPF_NS):
            href = hrefs.get(itemref.get("idref"))
            if href is None:
                continue
            document_path = posixpath.normpath(
                posixpath.join(package_dir, urllib.parse.unquote(href))
            )
            if document_path.startswith("../") or document_path not in names:
                raise ValueError("EPUB spine document path is invalid")
            document = ElementTree.fromstring(archive.read(document_path))
            body = document.find(".//{%s}body" % XHTML_NS)
            if body is None:
                raise ValueError("EPUB spine document has no XHTML body")
            document_pieces = [piece for piece in body.itertext() if piece]
            visible_pieces.extend(document_pieces)
            xml_text_node_count += len(document_pieces)
            document_count += 1
        visible_text = "".join(visible_pieces)
    return visible_text, document_count, xml_text_node_count


def epub_visible_text(epub_path: Path) -> tuple[dict[str, Any], bool]:
    visible_text, document_count, xml_text_node_count = epub_visible_text_stream(epub_path)
    summary = summarize_text(visible_text)
    summary["document_count"] = document_count
    summary["xml_text_node_count"] = xml_text_node_count
    return summary, True


def stage_summary(
    segment_count: int | None,
    node_count: int | None,
    scalar_count: int | None,
    digest: str | None,
) -> dict[str, Any]:
    return {
        "text_segment_count": segment_count,
        "text_node_count": node_count,
        "unicode_scalar_count": scalar_count,
        "normalized_sha256_audit_only": digest,
    }


def pairwise_comparison(left: dict[str, Any], right: dict[str, Any]) -> dict[str, bool]:
    return {
        "unicode_scalar_count_equal": left["unicode_scalar_count"]
        == right["unicode_scalar_count"],
        "normalized_text_hash_equal": left["normalized_sha256_audit_only"]
        == right["normalized_sha256_audit_only"],
    }


def paired_unit_summary(
    left_units: list[dict[str, Any]],
    right_units: list[dict[str, Any]],
    key: str,
) -> dict[str, int]:
    left_by_order = {
        unit[key]: unit for unit in left_units if unit.get(key) is not None
    }
    right_by_order = {
        unit[key]: unit for unit in right_units if unit.get(key) is not None
    }
    shared_orders = left_by_order.keys() & right_by_order.keys()
    return {
        "left_unit_count": len(left_by_order),
        "right_unit_count": len(right_by_order),
        "paired_unit_count": len(shared_orders),
        "exact_hash_match_count": sum(
            left_by_order[order].get("normalized_sha256_audit_only")
            == right_by_order[order].get("normalized_sha256_audit_only")
            and left_by_order[order].get("unicode_scalar_count")
            == right_by_order[order].get("unicode_scalar_count")
            for order in shared_orders
        ),
        "mismatched_hash_count": sum(
            left_by_order[order].get("normalized_sha256_audit_only")
            != right_by_order[order].get("normalized_sha256_audit_only")
            or left_by_order[order].get("unicode_scalar_count")
            != right_by_order[order].get("unicode_scalar_count")
            for order in shared_orders
        ),
    }


def calibre_to_native_fragment_alignment(
    calibre_text: str,
    native_units: list[dict[str, Any]],
    native_scalar_count: int,
) -> dict[str, Any]:
    offset = 0
    exact_unit_count = 0
    mismatched_unit_count = 0
    exact_scalar_count = 0
    first_mismatch_order = None
    native_unit_scalar_total = sum(
        unit.get("unicode_scalar_count", 0) for unit in native_units
    )
    for unit in native_units:
        scalar_count = unit.get("unicode_scalar_count", 0)
        candidate = calibre_text[offset : offset + scalar_count]
        candidate_summary = summarize_text(candidate)
        exact = (
            len(candidate) == scalar_count
            and candidate_summary["normalized_sha256_audit_only"]
            == unit.get("normalized_sha256_audit_only")
        )
        if exact:
            exact_unit_count += 1
            exact_scalar_count += scalar_count
        else:
            mismatched_unit_count += 1
            if first_mismatch_order is None:
                first_mismatch_order = unit.get("content_fragment_order")
        offset += scalar_count
    return {
        "native_unit_count": len(native_units),
        "native_unit_scalar_total": native_unit_scalar_total,
        "native_book_scalar_count": native_scalar_count,
        "unit_boundaries_reconcile_with_book_total": native_unit_scalar_total
        == native_scalar_count,
        "calibre_oracle_scalar_count": len(calibre_text),
        "calibre_slice_consumed_scalar_count": min(offset, len(calibre_text)),
        "exact_unit_count": exact_unit_count,
        "mismatched_unit_count": mismatched_unit_count,
        "exact_scalar_count": exact_scalar_count,
        "first_mismatched_content_fragment_order": first_mismatch_order,
    }


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    entries = manifest.get("inputs")
    if not isinstance(entries, list) or len(entries) != 82:
        raise ValueError("anonymous text-audit manifest is not the expected 82 inputs")

    matches: dict[str, list[Path]] = {item["sha256"]: [] for item in entries}
    for path in args.book_root.iterdir():
        if (
            path.is_file()
            and not path.is_symlink()
            and path.suffix.lower() == ".kfx"
        ):
            digest = file_sha256(path)
            if digest in matches:
                matches[digest].append(path)
    if any(len(paths) != 1 for paths in matches.values()):
        raise ValueError("a pinned private corpus input did not resolve uniquely")

    calibre_version = subprocess.run(
        ["calibre-debug", "--version"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.strip()
    summaries = []
    for item in entries:
        book_path = matches[item["sha256"]][0]
        audit_path = args.fidelity_report_dir / f"{item['id']}.json"
        fidelity = json.loads(audit_path.read_text(encoding="utf-8"))
        if fidelity.get("input_sha256_audit_only") != item["sha256"]:
            raise ValueError("paired FolioForge text audit hash did not reconcile")

        raw_path = args.temp_dir / f"{item['id']}.raw.json"
        result = subprocess.run(
            [
                "calibre-debug",
                "-r",
                "KFX Input",
                "--",
                "--json-content",
                str(book_path),
                str(raw_path),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=300,
        )
        try:
            if result.returncode != 0 or not raw_path.is_file():
                raise RuntimeError("KFX Input oracle failed")
            calibre_raw = json.loads(raw_path.read_text(encoding="utf-8"))
            calibre_visible_text, calibre_fragment_count = ordered_calibre_text(
                calibre_raw
            )
            calibre_text = summarize_text(calibre_visible_text)
            calibre_text["text_fragment_count"] = calibre_fragment_count
            del calibre_raw
        finally:
            raw_path.unlink(missing_ok=True)

        ir_text = fidelity.get("text", {})
        if not isinstance(ir_text.get("source_fragments"), list) or not isinstance(
            ir_text.get("semantic_documents"), list
        ):
            raise ValueError("FolioForge audit lacks the C2 stage-text trace")
        native_text = stage_summary(
            ir_text.get("source_text_segment_count"),
            None,
            ir_text.get("source_unicode_scalar_count"),
            ir_text.get("source_normalized_sha256_audit_only"),
        )
        semantic_text = stage_summary(
            ir_text.get("semantic_text_segment_count"),
            None,
            ir_text.get("semantic_unicode_scalar_count"),
            ir_text.get("semantic_normalized_sha256_audit_only"),
        )
        ir_stage = stage_summary(
            None,
            ir_text.get("text_node_count"),
            ir_text.get("unicode_scalar_count"),
            ir_text.get("normalized_sha256_audit_only"),
        )
        epub_path = args.temp_dir / f"{item['id']}.epub"
        try:
            subprocess.run(
                [
                    str(args.folio),
                    "convert",
                    str(book_path),
                    "--to",
                    "epub",
                    "--output",
                    str(epub_path),
                ],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=600,
            )
            epub_text, epub_valid = epub_visible_text(epub_path)
            validation = subprocess.run(
                [str(args.folio), "validate", str(epub_path)],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=120,
            )
            epub_valid = epub_valid and validation.returncode == 0
        finally:
            epub_path.unlink(missing_ok=True)
        calibre_stage = stage_summary(
            calibre_text["text_fragment_count"],
            None,
            calibre_text["unicode_scalar_count"],
            calibre_text["normalized_sha256_audit_only"],
        )
        source_units = ir_text["source_fragments"]
        semantic_units = ir_text["semantic_documents"]
        ir_units = ir_text.get("ir_documents", [])
        summaries.append(
            {
                "input_id": item["id"],
                "stages": {
                    "calibre_oracle": calibre_stage,
                    "native_kfx_text": native_text,
                    "semantic_text": semantic_text,
                    "folio_ir": ir_stage,
                    "epub_visible_text": {
                        "text_segment_count": None,
                        "text_node_count": epub_text["xml_text_node_count"],
                        "unicode_scalar_count": epub_text["unicode_scalar_count"],
                        "normalized_sha256_audit_only": epub_text[
                            "normalized_sha256_audit_only"
                        ],
                        "document_count": epub_text["document_count"],
                        "validation_passed": epub_valid,
                    },
                },
                "stage_parity": {
                    "calibre_vs_native": pairwise_comparison(calibre_stage, native_text),
                    "calibre_vs_semantic": pairwise_comparison(calibre_stage, semantic_text),
                    "calibre_vs_ir": pairwise_comparison(calibre_stage, ir_stage),
                    "calibre_vs_epub": pairwise_comparison(calibre_stage, epub_text),
                    "ir_vs_epub": pairwise_comparison(ir_stage, epub_text),
                },
                "source_fragment_vs_semantic_document_units": paired_unit_summary(
                    source_units, semantic_units, "content_fragment_order"
                ),
                "calibre_oracle_vs_native_fragment_alignment": calibre_to_native_fragment_alignment(
                    calibre_visible_text,
                    source_units,
                    ir_text.get("source_unicode_scalar_count", 0),
                ),
                "semantic_document_vs_ir_document_units": paired_unit_summary(
                    semantic_units, ir_units, "document_order"
                ),
            }
        )
        print(f"Compared text stages for {item['id']}.", flush=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "calibre_version": calibre_version,
                "kfx_input_version": "2.34.2",
                "comparison": "anonymous-calibre-native-semantic-ir-epub-text-stage-parity",
                "normalization": "Calibre type-1 text is concatenated in emitted order by generated cumulative PID; FolioForge stages use KFX reading-order content-fragment order and source text traversal; CRLF/CR to LF; Unicode NFC; SHA-256 over normalized UTF-8; Unicode scalar count",
                "fragment_alignment": "Diagnostic only: each native content-fragment hash is compared with an adjacent Calibre text slice using the native fragment's normalized scalar length; it does not claim Calibre exposes matching fragment boundaries.",
                "books": summaries,
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
    parser.add_argument("--fidelity-report-dir", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
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
        RuntimeError,
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        json.JSONDecodeError,
    ) as error:
        parser.error(
            "visible-text comparison failed ({}); source text and filenames were not emitted".format(
                type(error).__name__
            )
        )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
