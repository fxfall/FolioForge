#!/usr/bin/env python3
"""Run privacy-safe KFX-C1 audits over direct .kfx children of a corpus folder."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path


DEFAULT_OUTPUT = Path("tests-private/kfx-corpus")
STATUS_VALUES = {"Complete", "CompleteWithWarnings", "Partial", "Failed"}


def find_inputs(directory: Path) -> list[tuple[Path, int, str]]:
    inputs = []
    for path in directory.iterdir():
        if path.is_symlink() or not path.is_file():
            continue
        if path.suffix.lower() != ".kfx":
            continue
        digest = hashlib.sha256()
        size = 0
        with path.open("rb") as source:
            while chunk := source.read(1024 * 1024):
                size += len(chunk)
                digest.update(chunk)
        inputs.append((path, size, digest.hexdigest()))
    # Content hashes, rather than source paths or titles, define stable IDs.
    inputs.sort(key=lambda item: (item[2], item[1]))
    return inputs


def failed_report(digest: str, size: int, code: str) -> dict:
    return {
        "schema_version": 1,
        "input_sha256_audit_only": digest,
        "input_bytes": size,
        "status": "Failed",
        "error_code": code,
        "diagnostics": {code.lower(): 1},
    }


def add_counts(target: dict[str, int], values: object) -> None:
    if not isinstance(values, dict):
        return
    for key, value in values.items():
        if isinstance(value, int) and not isinstance(value, bool):
            target[key] = target.get(key, 0) + value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--folio", type=Path, default=Path("target/debug/folio"))
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args()

    try:
        sources = find_inputs(args.corpus)
    except OSError:
        print("Could not read the configured corpus directory.", file=sys.stderr)
        return 2
    if not sources:
        print("No direct .kfx files were found in the configured corpus directory.", file=sys.stderr)
        return 2
    if not args.folio.is_file():
        print("The FolioForge CLI binary is missing; build the workspace first.", file=sys.stderr)
        return 2

    reports_dir = args.output / "reports"
    reports_dir.mkdir(parents=True, exist_ok=True)
    manifest_books = []
    statuses: Counter[str] = Counter()
    totals: dict[str, int] = {}
    node_totals: dict[str, int] = {}
    resource_kind_totals: dict[str, int] = {}
    diagnostic_totals: dict[str, int] = {}

    for index, (path, source_size, digest) in enumerate(sources, start=1):
        started = time.perf_counter()
        try:
            result = subprocess.run(
                [str(args.folio), "inspect", str(path), "--kfx-fidelity-audit"],
                check=False,
                capture_output=True,
                text=True,
                timeout=180,
            )
            report = json.loads(result.stdout) if result.returncode == 0 else None
            if not isinstance(report, dict):
                report = failed_report(digest, source_size, "AUDIT_COMMAND_FAILED")
            elif report.get("input_sha256_audit_only") != digest:
                report = failed_report(digest, source_size, "AUDIT_INPUT_CHANGED")
        except (OSError, subprocess.TimeoutExpired, json.JSONDecodeError):
            report = failed_report(digest, source_size, "AUDIT_COMMAND_FAILED")
        report["audit_elapsed_ms"] = round((time.perf_counter() - started) * 1000)

        status = report.get("status")
        if status not in STATUS_VALUES:
            status = "Failed"
            report["status"] = status
            report["error_code"] = "AUDIT_INVALID_REPORT"
        statuses[status] += 1

        source_summary = report.get("source", {})
        native_summary = report.get("native", {})
        content_summary = report.get("content", {})
        resource_summary = report.get("resources", {})
        font_summary = report.get("fonts", {})
        if not isinstance(source_summary, dict):
            source_summary = {}
        if not isinstance(native_summary, dict):
            native_summary = {}
        if not isinstance(content_summary, dict):
            content_summary = {}
        if not isinstance(resource_summary, dict):
            resource_summary = {}
        if not isinstance(font_summary, dict):
            font_summary = {}

        corpus_id = f"KFX-C{index:03}"
        report_path = reports_dir / f"{corpus_id}.json"
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        manifest_books.append(
            {
                "id": corpus_id,
                "source_sha256_audit_only": digest,
                "source_bytes": source_size,
                "file_count": 1,
                "container_count": native_summary.get("container_count", 0),
                "kfx_variant": {
                    "format": source_summary.get("format", "unknown"),
                    "container_version_counts": source_summary.get("container_version_counts", {}),
                },
                "language": source_summary.get("language"),
                "source_layout": source_summary.get("layout", "unknown_not_decoded"),
                "resource_count": resource_summary.get("ir_resource_count", 0),
                "font_count": {
                    "declared_metadata_records": font_summary.get("declared_font_resource_record_count", 0),
                    "ir_resources": font_summary.get("font_resource_count", 0),
                    "mapped_faces": font_summary.get("mapped_font_resource_count", 0),
                },
                "chapter_count": content_summary.get("document_count", 0),
                "known_special_features": content_summary.get("special_feature_counts", {}),
                "status": status,
                "report": f"reports/{corpus_id}.json",
            }
        )

        for section, fields in (
            ("native", ("container_count", "entity_count", "fragment_count", "symbol_count", "string_table_count", "raw_resource_count", "unknown_fragment_count")),
            ("content", ("document_count", "content_fragment_count")),
            (
                "text",
                (
                    "text_node_count",
                    "unicode_scalar_count",
                    "source_text_segment_count",
                    "unresolved_text_reference_count",
                    "source_unicode_scalar_count",
                    "semantic_text_segment_count",
                    "semantic_unicode_scalar_count",
                ),
            ),
            ("links", ("link_node_count", "anchor_node_count", "anchor_count", "anchor_graph_edge_count")),
            ("navigation", ("toc_point_count", "landmark_point_count", "page_list_point_count")),
            ("resources", ("ir_resource_count",)),
            ("placements", ("raw_reference_occurrence_count", "native_occurrence_count", "semantic_occurrence_count", "ir_image_node_count", "exact_image_identity_count", "unresolved_or_ambiguous_identity_count")),
            ("styles", ("style_count", "style_property_count", "distinct_style_property_count", "nodes_with_nondefault_style_count")),
            ("fonts", ("declared_font_resource_record_count", "font_resource_count", "font_face_count", "mapped_font_resource_count")),
        ):
            values = report.get(section, {})
            if isinstance(values, dict):
                for field in fields:
                    value = values.get(field)
                    if isinstance(value, int) and not isinstance(value, bool):
                        key = f"{section}.{field}"
                        totals[key] = totals.get(key, 0) + value
        add_counts(node_totals, report.get("content", {}).get("ir_node_counts", {}))
        add_counts(resource_kind_totals, report.get("resources", {}).get("resource_kind_counts", {}))
        add_counts(diagnostic_totals, report.get("diagnostics", {}))

        if index % 10 == 0 or index == len(sources):
            print(f"Audited {index}/{len(sources)} inputs.", flush=True)

    manifest = {
        "schema_version": 1,
        "audit": "KFX-C1 post-fix corpus audit",
        "parser_version": "folioforge-kfx-cont/0.5-fidelity-audit",
        "corpus_count": len(manifest_books),
        "status_counts": dict(sorted(statuses.items())),
        "books": manifest_books,
    }
    summary = {
        "schema_version": 1,
        "corpus_count": len(manifest_books),
        "status_counts": dict(sorted(statuses.items())),
        "totals": dict(sorted(totals.items())),
        "ir_node_counts": dict(sorted(node_totals.items())),
        "resource_kind_counts": dict(sorted(resource_kind_totals.items())),
        "diagnostic_counts": dict(sorted(diagnostic_totals.items())),
        "privacy": "No source paths, filenames, titles, resource paths, or text are emitted.",
    }
    (args.output / "corpus.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    )
    (args.output / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        f"C1 audit complete: {len(manifest_books)} input(s); "
        f"Complete={statuses['Complete']}, "
        f"CompleteWithWarnings={statuses['CompleteWithWarnings']}, "
        f"Partial={statuses['Partial']}, Failed={statuses['Failed']}.",
        flush=True,
    )
    print(f"Private reports: {args.output}", flush=True)
    return 0 if statuses["Failed"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
