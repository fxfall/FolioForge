#!/usr/bin/env python3
"""Audit repetition of FolioForge-only blocks without persisting source text."""

from __future__ import annotations

import argparse
import collections
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


DIFF_PATH = Path(__file__).with_name("run-c2-text-diff.py")
DIFF_SPEC = importlib.util.spec_from_file_location("run_c2_text_diff_for_extra_blocks", DIFF_PATH)
DIFF = importlib.util.module_from_spec(DIFF_SPEC)
DIFF_SPEC.loader.exec_module(DIFF)


def count_nonoverlapping(haystack: str, needle: str) -> int:
    if not needle:
        return 0
    count = 0
    cursor = 0
    while True:
        found = haystack.find(needle, cursor)
        if found < 0:
            return count
        count += 1
        cursor = found + len(needle)


def block_categories(value: str) -> dict[str, int]:
    counts = collections.Counter(DIFF.category_name(char) for char in value)
    return dict(sorted(counts.items()))


def validate_pinned_streams(report: dict[str, Any], oracle_text: str, folio_text: str) -> list[dict[str, Any]]:
    comparison = report["comparison"]
    for side, text in (("oracle", oracle_text), ("folioforge", folio_text)):
        expected = comparison[side]
        if len(text) != expected["raw_unicode_scalar_count"]:
            raise ValueError(f"{side} private stream scalar count did not match the pinned C2 report")
        if DIFF.sha256_text(text) != expected["raw_sha256_audit_only"]:
            raise ValueError(f"{side} private stream digest did not match the pinned C2 report")
    hunks = comparison["hunks"]
    delta = sum(hunk["folio_len"] - hunk["oracle_len"] for hunk in hunks)
    if delta != len(folio_text) - len(oracle_text):
        raise ValueError("pinned hunk spans did not reconcile to the private stream delta")
    for hunk in hunks:
        if hunk["oracle_start"] + hunk["oracle_len"] > len(oracle_text):
            raise ValueError("pinned Oracle hunk exceeds the verified private stream")
        if hunk["folio_start"] + hunk["folio_len"] > len(folio_text):
            raise ValueError("pinned FolioForge hunk exceeds the verified private stream")
    return hunks


def repeated_block_summary(
    block: str,
    source_event_ids: set[str],
    folio_text: str,
    folio_events: list[dict[str, Any]],
    oracle_text: str,
    digest_frequency: int,
) -> dict[str, Any]:
    per_event = []
    for event in folio_events:
        count = count_nonoverlapping(event["text"], block)
        if count:
            per_event.append(
                {
                    "trace_id": event["trace_id"],
                    "source_fragment": event["source_fragment"],
                    "section_id": event.get("section_id"),
                    "story_id": event.get("story_id"),
                    "source_event": event["trace_id"] in source_event_ids,
                    "occurrence_count": count,
                }
            )
    other_events = [event for event in per_event if not event["source_event"]]
    return {
        "unicode_scalar_length": len(block),
        "sha256_raw_audit_only": hashlib.sha256(block.encode("utf-8")).hexdigest(),
        "scalar_categories": block_categories(block),
        "same_digest_extra_hunk_count": digest_frequency,
        "occurrences_in_native_stream_nonoverlapping": count_nonoverlapping(folio_text, block),
        "occurrences_in_oracle_stream_nonoverlapping": count_nonoverlapping(oracle_text, block),
        "native_events_containing_block": len(per_event),
        "other_native_event_occurrence_count": sum(event["occurrence_count"] for event in other_events),
        "other_native_events": other_events,
    }


def analyze_book(report: dict[str, Any], source: Path, folio: Path) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="folio-c2r-extra-block-") as temporary:
        calibre_path = Path(temporary) / "oracle.json"
        oracle_process = subprocess.run(
            [
                "calibre-debug",
                "-r",
                "KFX Input",
                "--",
                "--json-content",
                str(source),
                str(calibre_path),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=300,
        )
        if oracle_process.returncode != 0 or not calibre_path.is_file():
            raise RuntimeError(f"Calibre text oracle failed for {report['input_id']}")
        oracle_raw = json.loads(calibre_path.read_text(encoding="utf-8"))
        oracle_text, _oracle_events = DIFF.raw_oracle_events(oracle_raw)
        del oracle_raw

        native_process = subprocess.run(
            [
                str(folio),
                "inspect",
                str(source),
                "--kfx-text-event-audit",
                "--include-private-text",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            timeout=600,
        )
        if native_process.returncode != 0:
            raise RuntimeError(f"FolioForge private text audit failed for {report['input_id']}")
        native_audit = json.loads(native_process.stdout)
        if native_audit.get("input_sha256_audit_only") != report["source_sha256_audit_only"]:
            raise ValueError("private text audit did not match the pinned source digest")
        folio_text, folio_events = DIFF.native_event_stream(native_audit)
        del native_process

    hunks = validate_pinned_streams(report, oracle_text, folio_text)

    extra_rows = []
    for hunk_index, hunk in enumerate(hunks, 1):
        if hunk["folio_len"] <= hunk["oracle_len"]:
            continue
        block = folio_text[hunk["folio_start"] : hunk["folio_start"] + hunk["folio_len"]]
        provenance = hunk["folio_source_provenance"]
        source_events = provenance.get("events", [])
        extra_rows.append(
            {
                "hunk_index": hunk_index,
                "operation": hunk["kind"],
                "signature": hunk["signature"],
                "oracle_span_length": hunk["oracle_len"],
                "folio_span_length": hunk["folio_len"],
                "folio_minus_oracle_span_delta": hunk["folio_len"] - hunk["oracle_len"],
                "source_events": [
                    {
                        key: event.get(key)
                        for key in (
                            "trace_id",
                            "source_fragment",
                            "source_path",
                            "source_value_kind",
                            "section_id",
                            "story_id",
                        )
                    }
                    for event in hunk["folio_source_provenance"].get("events", [])
                ],
                "private_block": block,
            }
        )

    frequencies = collections.Counter(
        hashlib.sha256(row["private_block"].encode("utf-8")).hexdigest()
        for row in extra_rows
    )
    output_hunks = []
    for row in extra_rows:
        block = row.pop("private_block")
        source_event_ids = {event["trace_id"] for event in row["source_events"]}
        output_hunks.append(
            {
                **row,
                "repetition": repeated_block_summary(
                    block,
                    source_event_ids,
                    folio_text,
                    folio_events,
                    oracle_text,
                    frequencies[hashlib.sha256(block.encode("utf-8")).hexdigest()],
                ),
            }
        )

    return {
        "input_id": report["input_id"],
        "source_sha256_audit_only": report["source_sha256_audit_only"],
        "book_delta_folio_minus_oracle": len(folio_text) - len(oracle_text),
        "extra_span_hunk_count": len(output_hunks),
        "extra_span_scalar_delta_sum": sum(row["folio_minus_oracle_span_delta"] for row in output_hunks),
        "root_cause_status": "UnknownRootCause",
        "hunks": output_hunks,
    }


def positive_delta_reports(report_dir: Path) -> list[dict[str, Any]]:
    reports = []
    for path in sorted(report_dir.glob("KFX-C*/text-diff.json")):
        report = json.loads(path.read_text(encoding="utf-8"))
        comparison = report["comparison"]
        delta = comparison["folioforge"]["raw_unicode_scalar_count"] - comparison["oracle"]["raw_unicode_scalar_count"]
        if delta > 0:
            reports.append(report)
    if len(reports) != 10:
        raise ValueError("the pinned C2 reports did not resolve to ten FolioForge-extra cases")
    return sorted(
        reports,
        key=lambda report: (
            abs(
                report["comparison"]["folioforge"]["raw_unicode_scalar_count"]
                - report["comparison"]["oracle"]["raw_unicode_scalar_count"]
            ),
            report["input_id"],
        ),
    )


def resolve_sources(reports: list[dict[str, Any]], book_root: Path) -> dict[str, Path]:
    wanted = {report["source_sha256_audit_only"] for report in reports}
    found: dict[str, list[Path]] = {digest: [] for digest in wanted}
    for path in book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = DIFF.file_sha256(path)
            if digest in found:
                found[digest].append(path)
    if any(len(paths) != 1 for paths in found.values()):
        raise ValueError("a pinned FolioForge-extra case did not resolve uniquely")
    return {digest: paths[0] for digest, paths in found.items()}


def run(args: argparse.Namespace) -> int:
    reports = positive_delta_reports(args.report_dir)
    if args.only:
        requested = set(args.only)
        available = {report["input_id"] for report in reports}
        if requested - available:
            raise ValueError("requested ID is not a FolioForge-extra C2 case")
        reports = [report for report in reports if report["input_id"] in requested]
    sources = resolve_sources(reports, args.book_root)

    books = []
    for index, report in enumerate(reports, 1):
        print(f"[{index}/{len(reports)}] auditing extra blocks for {report['input_id']}", flush=True)
        books.append(analyze_book(report, sources[report["source_sha256_audit_only"]], args.folio))

    output = {
        "schema_version": 1,
        "scope": "repetition analysis of FolioForge-longer hunk spans; content is analyzed in memory only",
        "book_count": len(books),
        "extra_span_hunk_count": sum(book["extra_span_hunk_count"] for book in books),
        "extra_span_scalar_delta_sum": sum(book["extra_span_scalar_delta_sum"] for book in books),
        "books": books,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("wrote content-free extra-block repetition report", flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", type=Path, required=True)
    result.add_argument("--folio", type=Path, required=True)
    result.add_argument("--report-dir", type=Path, default=Path("tests-private/kfx-corpus/c2-r"))
    result.add_argument("--output", type=Path, default=Path("tests-private/kfx-corpus/c2-r/extra-block-analysis.json"))
    result.add_argument("--only", action="append", default=[])
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"C2 extra-block audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
