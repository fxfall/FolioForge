#!/usr/bin/env python3
"""Compare Calibre, Bokō, and Folio EPUB visible text with one extractor."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any


COMPARATOR_PATH = Path(__file__).with_name("compare-calibre-text.py")
COMPARATOR_SPEC = importlib.util.spec_from_file_location(
    "compare_calibre_text_for_boko", COMPARATOR_PATH
)
COMPARATOR = importlib.util.module_from_spec(COMPARATOR_SPEC)
COMPARATOR_SPEC.loader.exec_module(COMPARATOR)


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def epub_stream_summary(path: Path) -> dict[str, Any]:
    visible_text, document_count, xml_text_node_count = (
        COMPARATOR.epub_visible_text_stream(path)
    )
    normalized = COMPARATOR.summarize_text(visible_text)
    return {
        "raw_unicode_scalar_count": len(visible_text),
        "raw_sha256_audit_only": hashlib.sha256(
            visible_text.encode("utf-8")
        ).hexdigest(),
        "normalized_unicode_scalar_count": normalized["unicode_scalar_count"],
        "normalized_sha256_audit_only": normalized[
            "normalized_sha256_audit_only"
        ],
        "document_count": document_count,
        "xml_text_node_count": xml_text_node_count,
    }


def pairwise(left: dict[str, Any], right: dict[str, Any]) -> dict[str, bool]:
    return {
        "raw_scalar_count_equal": left["raw_unicode_scalar_count"]
        == right["raw_unicode_scalar_count"],
        "raw_hash_equal": left["raw_sha256_audit_only"]
        == right["raw_sha256_audit_only"],
        "normalized_scalar_count_equal": left["normalized_unicode_scalar_count"]
        == right["normalized_unicode_scalar_count"],
        "normalized_hash_equal": left["normalized_sha256_audit_only"]
        == right["normalized_sha256_audit_only"],
    }


def version(command: list[str]) -> str:
    result = subprocess.run(
        command,
        capture_output=True,
        text=True,
        check=True,
        timeout=30,
    )
    return (result.stdout or result.stderr).strip()


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    c2_comparison = json.loads(args.comparison.read_text(encoding="utf-8"))
    entries = manifest.get("books")
    c2_books = c2_comparison.get("books")
    if not isinstance(entries, list) or len(entries) != 82:
        raise ValueError("pinned anonymous corpus must contain 82 books")
    if not isinstance(c2_books, list) or len(c2_books) != 82:
        raise ValueError("C2 Calibre comparison must contain 82 books")

    selected = [
        row["input_id"]
        for row in c2_books
        if row["stage_parity"]["calibre_vs_native"]["unicode_scalar_count_equal"]
        and not row["stage_parity"]["calibre_vs_native"]["normalized_text_hash_equal"]
    ]
    if len(selected) != 20:
        raise ValueError("C2 equal-length mismatch class did not reconcile to 20")
    if args.only:
        unknown = set(args.only) - set(selected)
        if unknown:
            raise ValueError("requested anonymous ID is not in the 20-book class")
        selected = [book_id for book_id in selected if book_id in set(args.only)]

    c2r_root = args.c2r_dir
    for book_id in selected:
        report_path = c2r_root / book_id / "text-diff.json"
        if not report_path.is_file():
            raise ValueError("C2-R hunk report is missing for a selected input")

    by_id = {book["id"]: book for book in entries}
    source_by_hash: dict[str, list[Path]] = {
        book["source_sha256_audit_only"]: [] for book in entries
    }
    for path in args.book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in source_by_hash:
                source_by_hash[digest].append(path)
    if any(len(paths) != 1 for paths in source_by_hash.values()):
        raise ValueError("a pinned corpus input did not resolve uniquely")

    if shutil.which("ebook-convert") is None:
        raise ValueError("Calibre ebook-convert is not available")
    calibre_version = version(["calibre-debug", "--version"])
    boko_version = version([str(args.boko), "--version"])
    args.temp_dir.mkdir(parents=True, exist_ok=True)
    results = []

    for book_id in selected:
        item = by_id[book_id]
        book_path = source_by_hash[item["source_sha256_audit_only"]][0]
        calibre_epub = args.temp_dir / f"{book_id}.calibre.epub"
        boko_epub = args.temp_dir / f"{book_id}.boko.epub"
        folio_epub = args.temp_dir / f"{book_id}.folio.epub"
        try:
            subprocess.run(
                ["ebook-convert", str(book_path), str(calibre_epub)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
                timeout=900,
            )
            subprocess.run(
                [str(args.boko), "convert", str(book_path), str(boko_epub)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
                timeout=900,
            )
            subprocess.run(
                [
                    str(args.folio),
                    "convert",
                    str(book_path),
                    "--to",
                    "epub",
                    "--output",
                    str(folio_epub),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=True,
                timeout=900,
            )

            calibre_text = epub_stream_summary(calibre_epub)
            boko_text = epub_stream_summary(boko_epub)
            folio_text = epub_stream_summary(folio_epub)
            native_c2r = json.loads(
                (c2r_root / book_id / "text-diff.json").read_text(encoding="utf-8")
            )["comparison"]
            calibre_json = next(
                row for row in c2_books if row["input_id"] == book_id
            )["stages"]["calibre_oracle"]
            calibre_json_epub_comparison = {
                "normalized_scalar_count_equal": calibre_json[
                    "unicode_scalar_count"
                ]
                == calibre_text["normalized_unicode_scalar_count"],
                "normalized_hash_equal": calibre_json[
                    "normalized_sha256_audit_only"
                ]
                == calibre_text["normalized_sha256_audit_only"],
                "normalized_scalar_delta_epub_minus_json": calibre_text[
                    "normalized_unicode_scalar_count"
                ]
                - calibre_json["unicode_scalar_count"],
            }
            results.append(
                {
                    "input_id": book_id,
                    "source_sha256_audit_only": item["source_sha256_audit_only"],
                    "calibre_json_content": {
                        "normalized_unicode_scalar_count": calibre_json[
                            "unicode_scalar_count"
                        ],
                        "normalized_sha256_audit_only": calibre_json[
                            "normalized_sha256_audit_only"
                        ],
                    },
                    "native_folioforge": {
                        "raw_unicode_scalar_count": native_c2r["folioforge"][
                            "raw_unicode_scalar_count"
                        ],
                        "raw_sha256_audit_only": native_c2r["folioforge"][
                            "raw_sha256_audit_only"
                        ],
                        "source_features": native_c2r["folioforge"]["source_features"],
                    },
                    "epub_visible_text": {
                        "calibre": calibre_text,
                        "boko": boko_text,
                        "folioforge": folio_text,
                    },
                    "pairwise": {
                        "calibre_json_content_vs_calibre_epub": calibre_json_epub_comparison,
                        "calibre_epub_vs_boko_epub": pairwise(calibre_text, boko_text),
                    "calibre_epub_vs_folio_epub": pairwise(calibre_text, folio_text),
                    "boko_epub_vs_folio_epub": pairwise(boko_text, folio_text),
                    },
                }
            )
        finally:
            calibre_epub.unlink(missing_ok=True)
            boko_epub.unlink(missing_ok=True)
            folio_epub.unlink(missing_ok=True)
        print(f"Compared three EPUB text streams for {book_id}.", flush=True)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "comparison": "Calibre/Boko/Folio EPUB body itertext in spine order",
                "primary": "raw Unicode scalar streams",
                "secondary": "CRLF/CR to LF, then NFC",
                "extractor": "shared neutral EPUB XHTML body/spine extractor",
                "calibre_version": calibre_version,
                "kfx_input_version": "2.34.2",
                "boko_version": boko_version,
                "boko_license": "GPL-3.0-or-later; external executable only",
                "selected_book_count": len(results),
                "expected_equal_length_class_count": 20,
                "books": results,
                "summary": {
                    "boko_equals_folio_raw_count": sum(
                        row["pairwise"]["boko_epub_vs_folio_epub"]["raw_hash_equal"]
                        for row in results
                    ),
                    "calibre_json_equals_calibre_epub_normalized_count": sum(
                        row["pairwise"]["calibre_json_content_vs_calibre_epub"][
                            "normalized_hash_equal"
                        ]
                        for row in results
                    ),
                    "calibre_differs_boko_folio_raw_count": sum(
                        not row["pairwise"]["calibre_epub_vs_boko_epub"]["raw_hash_equal"]
                        and not row["pairwise"]["calibre_epub_vs_folio_epub"]["raw_hash_equal"]
                        and row["pairwise"]["boko_epub_vs_folio_epub"]["raw_hash_equal"]
                        for row in results
                    ),
                },
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--comparison", required=True, type=Path)
    parser.add_argument("--c2r-dir", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--boko", required=True, type=Path)
    parser.add_argument("--temp-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--only", action="append", default=[])
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
        detail = (
            " field={!r}".format(error.args[0])
            if isinstance(error, KeyError) and error.args
            else ""
        )
        print(
            "C2-R three-implementation comparison failed ({}{}); no source text or filename was emitted.".format(
                type(error).__name__, detail
            ),
            file=sys.stderr,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
