#!/usr/bin/env python3
"""Correlate same-scalar-count C2 hunks with native adjacent style ranges."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

NOTE_REFERENCE_SYMBOL_ID = 617


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ordered_source_events(audit: dict[str, Any]) -> dict[str, dict[str, Any]]:
    events: dict[str, dict[str, Any]] = {}
    offset = 0
    rows = audit.get("events")
    if not isinstance(rows, list):
        raise ValueError("native text-event audit has no event list")
    for event in rows:
        if not isinstance(event, dict) or not isinstance(event.get("text_len"), int):
            raise ValueError("native text-event audit has an invalid event")
        if "text" in event:
            raise ValueError("content-free native audit unexpectedly contains body text")
        event = {**event, "start": offset, "end": offset + event["text_len"]}
        trace_id = event.get("trace_id")
        if not isinstance(trace_id, str) or trace_id in events:
            raise ValueError("native text-event IDs are missing or duplicated")
        events[trace_id] = event
        offset += event["text_len"]
    if audit.get("stream", {}).get("raw_unicode_scalar_count") != offset:
        raise ValueError("native text-event lengths do not reconcile to the stream")
    return events


def classify_hunk(
    hunk: dict[str, Any],
    source_events: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    provenance = hunk.get("folio_source_provenance", {}).get("events", [])
    exact_ranges = []
    symbol_617_overlaps = []
    considered_ranges = []
    unresolved_sources = []

    for source in provenance:
        trace_id = source.get("trace_id")
        event = source_events.get(trace_id)
        if (
            event is None
            or event.get("source_fragment") != source.get("source_fragment")
            or event.get("source_path") != source.get("source_path")
            or event.get("start") != source.get("start")
        ):
            unresolved_sources.append(trace_id)
            continue

        local_offset = hunk["folio_start"] - event["start"]
        span_length = hunk["folio_len"]
        event_end = event["end"]
        if local_offset < 0 or local_offset + span_length > event_end - event["start"]:
            unresolved_sources.append(trace_id)
            continue

        styles = event.get("adjacent_style_events", [])
        for style in styles:
            start = style.get("text_offset")
            length = style.get("text_length")
            symbol_id = style.get("style_symbol_id")
            if not isinstance(start, int) or not isinstance(length, int):
                continue
            record = {
                "trace_id": trace_id,
                "source_fragment": event["source_fragment"],
                "source_path": event["source_path"],
                "local_scalar_offset": local_offset,
                "folio_span_length": span_length,
                "style_list_index": style.get("list_index"),
                "style_offset": start,
                "style_length": length,
                "style_symbol_id": symbol_id,
            }
            considered_ranges.append(record)
            if start == local_offset and length == span_length:
                exact_ranges.append(record)
            if (
                symbol_id == NOTE_REFERENCE_SYMBOL_ID
                and max(start, local_offset) < min(start + length, local_offset + span_length)
            ):
                symbol_617_overlaps.append(record)

    if any(row["style_symbol_id"] == NOTE_REFERENCE_SYMBOL_ID for row in exact_ranges):
        classification = "exact_symbol_617_range"
    elif exact_ranges:
        classification = "exact_range_other_or_unresolved_symbol"
    elif symbol_617_overlaps:
        classification = "symbol_617_overlap_not_exact"
    elif unresolved_sources:
        classification = "source_event_unresolved"
    else:
        classification = "no_exact_symbol_617_range"

    return {
        "classification": classification,
        "exact_style_ranges": exact_ranges,
        "symbol_617_overlaps": symbol_617_overlaps,
        "source_style_ranges_considered": considered_ranges,
        "unresolved_trace_ids": unresolved_sources,
    }


def select_same_scalar_count_reports(report_dir: Path) -> list[dict[str, Any]]:
    selected = []
    for path in sorted(report_dir.glob("KFX-C*/text-diff.json")):
        report = json.loads(path.read_text(encoding="utf-8"))
        comparison = report.get("comparison", {})
        if comparison.get("oracle", {}).get("raw_unicode_scalar_count") == comparison.get(
            "folioforge", {}
        ).get("raw_unicode_scalar_count"):
            selected.append(report)
    if len(selected) != 20:
        raise ValueError("the pinned C2 reports did not resolve to 20 same-scalar-count cases")
    return selected


def resolve_sources(reports: list[dict[str, Any]], book_root: Path) -> dict[str, Path]:
    wanted = {report["source_sha256_audit_only"] for report in reports}
    found: dict[str, list[Path]] = {digest: [] for digest in wanted}
    for path in book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in found:
                found[digest].append(path)
    if any(len(paths) != 1 for paths in found.values()):
        raise ValueError("a pinned same-count corpus source did not resolve uniquely")
    return {digest: paths[0] for digest, paths in found.items()}


def audit_book(
    report: dict[str, Any], source: Path, folio: Path
) -> dict[str, Any]:
    process = subprocess.run(
        [str(folio), "inspect", str(source), "--kfx-text-event-audit"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
        timeout=600,
    )
    if process.returncode != 0:
        raise RuntimeError(f"content-free FolioForge audit failed for {report['input_id']}")
    audit = json.loads(process.stdout)
    if audit.get("input_sha256_audit_only") != report["source_sha256_audit_only"]:
        raise ValueError("native text audit did not match the pinned input digest")
    source_events = ordered_source_events(audit)

    rows = []
    classifications: Counter[str] = Counter()
    for hunk_index, hunk in enumerate(report["comparison"].get("hunks", []), 1):
        result = classify_hunk(hunk, source_events)
        classifications[result["classification"]] += 1
        rows.append(
            {
                "hunk_index": hunk_index,
                "kind": hunk["kind"],
                "folio_start": hunk["folio_start"],
                "folio_length": hunk["folio_len"],
                **result,
            }
        )

    return {
        "input_id": report["input_id"],
        "source_sha256_audit_only": report["source_sha256_audit_only"],
        "hunk_count": len(rows),
        "classification_counts": dict(sorted(classifications.items())),
        "hunks": rows,
    }


def run(args: argparse.Namespace) -> int:
    reports = select_same_scalar_count_reports(args.report_dir)
    if args.only:
        requested = set(args.only)
        available = {report["input_id"] for report in reports}
        if requested - available:
            raise ValueError("requested ID is not a same-scalar-count C2 case")
        reports = [report for report in reports if report["input_id"] in requested]
    sources = resolve_sources(reports, args.book_root)

    books = []
    for index, report in enumerate(reports, 1):
        print(f"[{index}/{len(reports)}] auditing {report['input_id']}", flush=True)
        books.append(
            audit_book(report, sources[report["source_sha256_audit_only"]], args.folio)
        )

    total_counts: Counter[str] = Counter()
    for book in books:
        total_counts.update(book["classification_counts"])
    output = {
        "schema_version": 1,
        "scope": "source range correlation for same-scalar-count C2 hunks; not a rendering-semantic verdict",
        "book_count": len(books),
        "hunk_count": sum(book["hunk_count"] for book in books),
        "classification_counts": dict(sorted(total_counts.items())),
        "books": books,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote content-free source correlations for {len(books)} books", flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", type=Path, required=True)
    result.add_argument("--folio", type=Path, required=True)
    result.add_argument(
        "--report-dir",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r"),
    )
    result.add_argument(
        "--output",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r/style-correlations.json"),
    )
    result.add_argument("--only", action="append", default=[])
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"C2 style correlation audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
