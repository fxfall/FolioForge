#!/usr/bin/env python3
"""Audit local DRM-free KFX conversion reports without printing book content."""

from __future__ import annotations

import argparse
from datetime import date
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


TARGET_EXTENSIONS = {"epub": ".epub", "kf7": ".mobi", "kf8": ".azw3"}


def file_id(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()[:16]


def run_conversion(
    binary: Path, source: Path, target: str, output: Path, timeout_seconds: int
) -> dict:
    try:
        result = subprocess.run(
            [
                str(binary),
                "convert",
                str(source),
                "--to",
                target,
                "--mode",
                "compatible",
                "--output",
                str(output),
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired:
        return {"target": target, "success": False, "error_class": "timeout"}
    if result.returncode != 0:
        error_text = result.stderr.lower()
        error_class = (
            "protected_input"
            if "drm" in error_text or "protected" in error_text
            else "conversion_error"
        )
        return {"target": target, "success": False, "error_class": error_class}

    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError:
        return {"target": target, "success": False, "error_class": "invalid_cli_report"}

    input_report = report.get("input_report", {})
    semantic_report = report.get("semantic_report", {})
    compatibility_report = report.get("compatibility_report", {})
    round_trip = report.get("round_trip", {})
    return {
        "target": target,
        "success": True,
        "output_size": report.get("output_size"),
        "output_validated": report.get("output_report", {}).get("validated"),
        "input_documents": input_report.get("document_count"),
        "input_resources": input_report.get("resource_count"),
        "input_unknown_feature_count": len(input_report.get("unknown_features", [])),
        "input_loss": input_report.get("input_loss", []),
        "semantic_valid": semantic_report.get("valid"),
        "semantic_documents": semantic_report.get("document_count"),
        "semantic_resources": semantic_report.get("resource_count"),
        "target_loss": compatibility_report.get("target_loss", []),
        "round_trip_checked": round_trip.get("checked"),
        "round_trip_passed": round_trip.get("passed"),
        "round_trip_unexpected_losses": round_trip.get("unexpected_losses", []),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "corpus",
        nargs="?",
        type=Path,
        default=Path(os.environ.get("FOLIOFORGE_KFX_CORPUS", "private-corpus")),
        help="external directory recursively containing DRM-free .kfx files",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path("target/debug/folio"),
        help="built FolioForge CLI executable",
    )
    parser.add_argument(
        "--targets",
        default="epub,kf7,kf8",
        help="comma-separated target IDs (default: epub,kf7,kf8)",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=int,
        default=180,
        help="maximum seconds per input/target conversion (default: 180)",
    )
    parser.add_argument(
        "--only-id",
        action="append",
        default=[],
        help="select one or more inputs by their existing anonymized hash ID",
    )
    parser.add_argument(
        "--report",
        type=Path,
        help="write the anonymized per-book JSON report to a new file (must not exist)",
    )
    args = parser.parse_args()

    root = args.corpus.expanduser().resolve()
    binary = args.binary.expanduser().resolve()
    targets = [value.strip().lower() for value in args.targets.split(",") if value.strip()]
    unknown_targets = sorted(set(targets) - TARGET_EXTENSIONS.keys())
    if unknown_targets:
        parser.error(f"unsupported audit target(s): {', '.join(unknown_targets)}")
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be a positive integer")
    if not root.is_dir():
        parser.error(f"corpus directory does not exist: {root}")
    if not binary.is_file():
        parser.error(f"CLI binary not found: {binary}; run cargo build -p folio-cli")

    sources = sorted(
        (path for path in root.rglob("*") if path.is_file() and path.suffix.lower() == ".kfx"),
        key=lambda path: str(path).casefold(),
    )
    if args.only_id:
        selected_ids = set(args.only_id)
        sources = [source for source in sources if file_id(source) in selected_ids]
        found_ids = {file_id(source) for source in sources}
        if found_ids != selected_ids:
            parser.error("one or more anonymized input IDs are not present in the corpus")
    if not sources:
        parser.error(f"no .kfx files found under {root}")

    books = []
    with tempfile.TemporaryDirectory(prefix="folioforge-kfx-conversions-") as temp_root:
        output_root = Path(temp_root)
        for index, source in enumerate(sources, start=1):
            source_id = file_id(source)
            conversions = []
            print(f"auditing input {index}/{len(sources)}", file=sys.stderr, flush=True)
            for target in targets:
                output = output_root / f"{index:04d}-{target}{TARGET_EXTENSIONS[target]}"
                conversions.append(
                    run_conversion(binary, source, target, output, args.timeout_seconds)
                )
            books.append(
                {
                    "book_id": source_id,
                    "conversions": conversions,
                }
            )
            if index == 1 or index % 10 == 0 or index == len(sources):
                print(f"audited {index}/{len(sources)} KFX inputs", file=sys.stderr, flush=True)

    summary = {}
    for target in targets:
        rows = [
            conversion
            for book in books
            for conversion in book["conversions"]
            if conversion["target"] == target
        ]
        successful = [row for row in rows if row["success"]]
        summary[target] = {
            "attempted": len(rows),
            "succeeded": len(successful),
            "failed": len(rows) - len(successful),
            "validated_outputs": sum(row.get("output_validated") is True for row in successful),
            "round_trip_passed": sum(row.get("round_trip_passed") is True for row in successful),
            "round_trip_failed": sum(row.get("round_trip_passed") is False for row in successful),
            "input_loss_items": sum(len(row.get("input_loss", [])) for row in successful),
            "target_loss_items": sum(len(row.get("target_loss", [])) for row in successful),
        }

    report = {
        "audit_date": date.today().isoformat(),
        "corpus_file_count": len(sources),
        "targets": targets,
        "summary": summary,
        "books": books,
        "book_content_emitted": False,
        "temporary_outputs_removed": True,
    }
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        report_path = args.report.expanduser().resolve()
        if report_path.exists():
            parser.error(f"refusing to overwrite existing report: {report_path}")
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
