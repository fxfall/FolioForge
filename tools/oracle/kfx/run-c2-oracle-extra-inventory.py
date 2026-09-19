#!/usr/bin/env python3
"""Match Oracle-only C2 text spans to content-free KFX source-string roles."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import platform
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any


DIFF_PATH = Path(__file__).with_name("run-c2-text-diff.py")
DIFF_SPEC = importlib.util.spec_from_file_location("run_c2_text_diff_for_inventory", DIFF_PATH)
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


def source_string_matches(block: str, entries: list[dict[str, Any]], limit: int = 200) -> dict[str, Any]:
    matches = []
    total = 0
    for entry in entries:
        source_text = entry.get("private_text")
        if not isinstance(source_text, str) or not source_text:
            continue
        occurrences = count_nonoverlapping(source_text, block)
        if not occurrences:
            continue
        total += 1
        if len(matches) >= limit:
            continue
        references = entry.get("references", [])
        content_references = [
            reference
            for reference in references
            if reference.get("role_candidate") == "content_fragment_string_reference_candidate"
        ]
        matches.append(
            {
                "string_id": entry.get("string_id"),
                "match_kind": "exact_string_value" if source_text == block else "substring_in_source_string",
                "occurrence_count_in_source_string": occurrences,
                "fragment_index": entry.get("fragment_index"),
                "fragment_type": entry.get("fragment_type"),
                "entity_id": entry.get("entity_id"),
                "content_fragment_alias": entry.get("content_fragment_alias"),
                "source_path": entry.get("source_path"),
                "parent_field_ids": entry.get("parent_field_ids", []),
                "role_candidate": entry.get("role_candidate"),
                "unicode_scalar_length": entry.get("unicode_scalar_length"),
                "sha256_raw_audit_only": entry.get("sha256_raw_audit_only"),
                "string_table_id": entry.get("string_table_id"),
                "string_table_index": entry.get("string_table_index"),
                "reference_count": entry.get("reference_count", 0),
                "content_fragment_reference_count": len(content_references),
                "content_fragment_references": [
                    {
                        key: reference.get(key)
                        for key in (
                            "fragment_index",
                            "fragment_type",
                            "entity_id",
                            "content_fragment_alias",
                            "source_path",
                            "parent_field_ids",
                            "role_candidate",
                        )
                    }
                    for reference in content_references[:12]
                ],
                "content_fragment_references_truncated": len(content_references) > 12,
            }
        )
    matches.sort(
        key=lambda match: (
            match["content_fragment_reference_count"] == 0,
            match["fragment_index"] if match["fragment_index"] is not None else -1,
            match["source_path"] or "",
        )
    )
    return {"match_count": total, "matches": matches, "truncated": total > limit}


def query_source_matches(
    query_id: str,
    entries: list[dict[str, Any]],
    limit: int = 200,
    already_filtered: bool = False,
    exact_length: int | None = None,
    exact_sha256: str | None = None,
) -> dict[str, Any]:
    selected = entries if already_filtered else [
        entry for entry in entries if query_id in entry.get("matched_query_ids", [])
    ]
    exact_match_count = (
        sum(
            entry.get("unicode_scalar_length") == exact_length
            and entry.get("sha256_raw_audit_only") == exact_sha256
            for entry in selected
        )
        if exact_length is not None and exact_sha256 is not None
        else 0
    )
    matches = []
    for entry in selected[:limit]:
        references = entry.get("references", [])
        content_references = [
            reference
            for reference in references
            if reference.get("role_candidate") == "content_fragment_string_reference_candidate"
        ]
        matches.append(
            {
                "string_id": entry.get("string_id"),
                "match_kind": "exact_string_value" if query_id in entry.get("exact_query_ids", []) else "substring_in_source_string",
                "fragment_index": entry.get("fragment_index"),
                "fragment_type": entry.get("fragment_type"),
                "entity_id": entry.get("entity_id"),
                "content_fragment_alias": entry.get("content_fragment_alias"),
                "source_path": entry.get("source_path"),
                "parent_field_ids": entry.get("parent_field_ids", []),
                "role_candidate": entry.get("role_candidate"),
                "unicode_scalar_length": entry.get("unicode_scalar_length"),
                "sha256_raw_audit_only": entry.get("sha256_raw_audit_only"),
                "string_table_id": entry.get("string_table_id"),
                "string_table_index": entry.get("string_table_index"),
                "reference_count": entry.get("reference_count", 0),
                "content_fragment_reference_count": len(content_references),
                "content_fragment_references": [
                    {
                        key: reference.get(key)
                        for key in (
                            "fragment_index",
                            "fragment_type",
                            "entity_id",
                            "content_fragment_alias",
                            "source_path",
                            "parent_field_ids",
                            "role_candidate",
                        )
                    }
                    for reference in content_references[:12]
                ],
                "content_fragment_references_truncated": len(content_references) > 12,
            }
        )
    matches.sort(
        key=lambda match: (
            match["content_fragment_reference_count"] == 0,
            match["fragment_index"] if match["fragment_index"] is not None else -1,
            match["source_path"] or "",
        )
    )
    return {
        "match_count": len(selected),
        "exact_match_count": exact_match_count,
        "matches": matches,
        "truncated": len(selected) > limit,
    }


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


def native_event_occurrences(block: str, events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    matches = []
    for event in events:
        count = count_nonoverlapping(event["text"], block)
        if count:
            matches.append(
                {
                    "trace_id": event["trace_id"],
                    "source_fragment": event["source_fragment"],
                    "source_path": event["source_path"],
                    "section_id": event.get("section_id"),
                    "story_id": event.get("story_id"),
                    "occurrence_count": count,
                }
            )
    return matches


def oracle_query_segments(
    hunk_index: int,
    hunk: dict[str, Any],
    oracle_text: str,
    events: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Split one Oracle hunk into source-event-aligned private queries.

    The query text is deliberately carried only in the returned in-memory
    objects. The caller writes it to a temporary file and removes it before
    persisting the report. A hunk with no event coverage is retained as an
    explicit bounded-coverage query so that the report distinguishes that
    condition from a covered query with no decoded source-string match.
    """
    start = int(hunk["oracle_start"])
    length = int(hunk["oracle_len"])
    end = start + length
    segments: list[dict[str, Any]] = []
    for event_index, event in enumerate(events):
        event_start = int(event["start"])
        event_end = event_start + int(event["length"])
        overlap_start = max(start, event_start)
        overlap_end = min(end, event_end)
        if overlap_start >= overlap_end:
            continue
        text = oracle_text[overlap_start:overlap_end]
        segments.append(
            {
                "query_id": f"H{hunk_index:06}E{len(segments) + 1:03}",
                "event_index": event_index,
                "oracle_start": overlap_start,
                "oracle_end": overlap_end,
                "oracle_length": overlap_end - overlap_start,
                "hunk_relative_start": overlap_start - start,
                "hunk_relative_end": overlap_end - start,
                "match_mode": "exact" if text.isspace() or len(text) <= 2 else "contains",
                "sha256_raw_audit_only": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                "oracle_event": {
                    "pid": event["pid"],
                    "row_order": event["row_order"],
                    "start": event["start"],
                    "length": event["length"],
                    "sha256_raw_audit_only": event["sha256_raw_audit_only"],
                    "overlap_start": overlap_start,
                    "overlap_end": overlap_end,
                    "overlap_length": overlap_end - overlap_start,
                },
                "private_text": text,
            }
        )
    if not segments and length > 0:
        text = oracle_text[start:end]
        segments.append(
            {
                "query_id": f"H{hunk_index:06}U001",
                "event_index": None,
                "oracle_start": start,
                "oracle_end": end,
                "oracle_length": length,
                "hunk_relative_start": 0,
                "hunk_relative_end": length,
                "match_mode": "exact" if text.isspace() or len(text) <= 2 else "contains",
                "sha256_raw_audit_only": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                "oracle_event": None,
                "private_text": text,
            }
        )
    return segments


def classify_source_query_segments(
    segment_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    """Classify query coverage without claiming reading-flow semantics."""
    if not segment_rows:
        return {
            "label": "empty_oracle_span",
            "oracle_event_segment_count": 0,
            "matched_segment_count": 0,
            "candidate_count": 0,
            "max_candidates_per_segment": 0,
            "basis": "the Oracle hunk had no positive-length event segment",
        }
    candidate_counts = [int(row["inventory"]["match_count"]) for row in segment_rows]
    matched_count = sum(count > 0 for count in candidate_counts)
    candidate_count = sum(candidate_counts)
    max_candidates = max(candidate_counts)
    if max_candidates > 1:
        label = "multiple_candidates"
        basis = "at least one event-aligned query matched multiple decoded source-string rows"
    elif matched_count == 0:
        label = (
            "covered_inventory_no_match"
            if all(row["oracle_event"] is not None for row in segment_rows)
            else "no_oracle_event_coverage"
        )
        basis = (
            "all event-aligned queries were covered by the decoded inventory but matched no row"
            if label == "covered_inventory_no_match"
            else "the hunk was outside the positive-length Oracle event inventory"
        )
    elif len(segment_rows) == 1:
        label = "single_node"
        basis = "one Oracle event segment has one decoded source-string candidate"
    elif matched_count == len(segment_rows):
        label = "continuous_multi_node"
        basis = "every contiguous Oracle event segment has one decoded source-string candidate"
    else:
        label = "partial_multi_node"
        basis = "only some contiguous Oracle event segments have a decoded source-string candidate"
    return {
        "label": label,
        "oracle_event_segment_count": len(segment_rows),
        "matched_segment_count": matched_count,
        "candidate_count": candidate_count,
        "max_candidates_per_segment": max_candidates,
        "basis": basis,
    }


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


def analyze_book(report: dict[str, Any], source: Path, folio: Path) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="folio-c2r-oracle-inventory-") as temporary:
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
        oracle_text, oracle_events = DIFF.raw_oracle_events(oracle_raw)
        del oracle_raw

        native_process = subprocess.run(
            [str(folio), "inspect", str(source), "--kfx-text-event-audit", "--include-private-text"],
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

        private_hunks = []
        native_events_by_id = {event["trace_id"]: event for event in folio_events}
        for hunk_index, hunk in enumerate(hunks, 1):
            if hunk["oracle_len"] <= hunk["folio_len"]:
                continue
            block = oracle_text[hunk["oracle_start"] : hunk["oracle_start"] + hunk["oracle_len"]]
            query_segments = oracle_query_segments(hunk_index, hunk, oracle_text, oracle_events)
            private_hunks.append(
                {
                    "hunk_index": hunk_index,
                    "operation": hunk["kind"],
                    "signature": hunk["signature"],
                    "oracle_start": hunk["oracle_start"],
                    "oracle_span_length": hunk["oracle_len"],
                    "folio_start": hunk["folio_start"],
                    "folio_span_length": hunk["folio_len"],
                    "oracle_minus_folio_span_delta": hunk["oracle_len"] - hunk["folio_len"],
                    "oracle_scalar_categories": hunk["oracle_classes"]["counts"],
                    "oracle_block_sha256_audit_only": hashlib.sha256(block.encode("utf-8")).hexdigest(),
                    "oracle_event_segment_count": len(query_segments),
                    "oracle_source_events": [
                        segment["oracle_event"]
                        for segment in query_segments
                        if segment["oracle_event"] is not None
                    ],
                    "oracle_query_segments": [
                        {
                            key: segment[key]
                            for key in (
                                "query_id",
                                "event_index",
                                "oracle_start",
                                "oracle_end",
                                "oracle_length",
                                "hunk_relative_start",
                                "hunk_relative_end",
                                "match_mode",
                                "sha256_raw_audit_only",
                                "oracle_event",
                            )
                        }
                        for segment in query_segments
                    ],
                    "folio_source_events": [
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
                        for event in hunk.get("folio_source_provenance", {}).get("events", [])
                    ],
                    "same_block_in_folio_provenance_events": native_event_occurrences(
                        block,
                        [
                            native_events_by_id[event["trace_id"]]
                            for event in hunk.get("folio_source_provenance", {}).get("events", [])
                            if event.get("trace_id") in native_events_by_id
                        ],
                    ),
                    "private_query_segments": query_segments,
                }
            )

        query_path = Path(temporary) / "oracle-only-queries.json"
        query_path.write_text(
            json.dumps(
                [
                    {
                        "query_id": segment["query_id"],
                        "text": segment["private_text"],
                        "match_mode": segment["match_mode"],
                    }
                    for hunk in private_hunks
                    for segment in hunk["private_query_segments"]
                ],
                ensure_ascii=False,
            ),
            encoding="utf-8",
        )

        inventory_process = subprocess.run(
            [
                str(folio),
                "inspect",
                str(source),
                "--kfx-source-string-audit",
                "--source-string-query-file",
                str(query_path),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            timeout=600,
        )
        if inventory_process.returncode != 0:
            raise RuntimeError(f"FolioForge private source-string audit failed for {report['input_id']}")
        inventory = json.loads(inventory_process.stdout)
        if inventory.get("input_sha256_audit_only") != report["source_sha256_audit_only"]:
            raise ValueError("source-string inventory did not match the pinned source digest")
        if inventory.get("private_strings_included") is not False:
            raise ValueError("query-mode inventory unexpectedly included private source strings")
        if inventory.get("inventory_mode") != "query_matches_only":
            raise ValueError("source-string query mode did not return a filtered inventory")
        del inventory_process
    inventory_rows_by_query: dict[str, list[dict[str, Any]]] = {}
    for entry in inventory.get("strings", []):
        for query_id in entry.get("matched_query_ids", []):
            inventory_rows_by_query.setdefault(query_id, []).append(entry)
    oracle_hunks = []
    classification_counts: dict[str, int] = {}
    matched_query_segment_count = 0
    query_segment_count = 0
    exact_source_candidate_row_count = 0
    exact_source_candidate_segment_count = 0
    for hunk in private_hunks:
        segment_rows = []
        all_matches = []
        seen_match_keys = set()
        hunk_candidate_row_count = 0
        hunk_reported_match_count = 0
        for segment in hunk["private_query_segments"]:
            inventory_match = query_source_matches(
                segment["query_id"],
                inventory_rows_by_query.get(segment["query_id"], []),
                limit=12,
                already_filtered=True,
                exact_length=segment["oracle_length"],
                exact_sha256=segment["sha256_raw_audit_only"],
            )
            segment_row = {
                key: value
                for key, value in segment.items()
                if key != "private_text"
            }
            segment_row["inventory"] = inventory_match
            segment_row["exact_source_candidate_count"] = inventory_match["exact_match_count"]
            exact_source_candidate_row_count += inventory_match["exact_match_count"]
            exact_source_candidate_segment_count += bool(inventory_match["exact_match_count"])
            hunk_candidate_row_count += inventory_match["match_count"]
            hunk_reported_match_count += len(inventory_match["matches"])
            segment_rows.append(segment_row)
            if inventory_match["match_count"] > 0:
                matched_query_segment_count += 1
            query_segment_count += 1
            for match in inventory_match["matches"]:
                match_key = (
                    match.get("string_id"),
                    match.get("fragment_index"),
                    match.get("source_path"),
                )
                if match_key not in seen_match_keys:
                    seen_match_keys.add(match_key)
                    all_matches.append(match)
        classification = classify_source_query_segments(segment_rows)
        classification_counts[classification["label"]] = classification_counts.get(
            classification["label"], 0
        ) + 1
        row = {
            key: value
            for key, value in hunk.items()
            if key != "private_query_segments"
        }
        row["oracle_source_event_count"] = len(row["oracle_source_events"])
        row["source_query_classification"] = classification
        row["oracle_query_segments"] = segment_rows
        row["raw_kfx_string_inventory"] = {
            "match_count": hunk_candidate_row_count,
            "reported_match_count": hunk_reported_match_count,
            "matches": all_matches[:200],
            "truncated": hunk_reported_match_count < hunk_candidate_row_count,
        }
        row["root_cause_status"] = "UnknownRootCause"
        oracle_hunks.append(row)
    del private_hunks

    return {
        "input_id": report["input_id"],
        "source_sha256_audit_only": report["source_sha256_audit_only"],
        "book_delta_folio_minus_oracle": len(folio_text) - len(oracle_text),
        "oracle_extra_span_hunk_count": len(oracle_hunks),
        "oracle_extra_span_scalar_delta_sum": sum(row["oracle_minus_folio_span_delta"] for row in oracle_hunks),
        "matched_source_string_count": inventory["string_count"],
        "unmatched_query_count": len(inventory.get("unmatched_query_ids", [])),
        "query_segment_count": query_segment_count,
        "matched_query_segment_count": matched_query_segment_count,
        "exact_source_candidate_row_count": exact_source_candidate_row_count,
        "exact_source_candidate_segment_count": exact_source_candidate_segment_count,
        "source_query_classification_counts": dict(sorted(classification_counts.items())),
        "root_cause_status": "UnknownRootCause",
        "hunks": oracle_hunks,
    }


def negative_delta_reports(report_dir: Path) -> list[dict[str, Any]]:
    reports = []
    for path in sorted(report_dir.glob("KFX-C*/text-diff.json")):
        report = json.loads(path.read_text(encoding="utf-8"))
        comparison = report["comparison"]
        delta = comparison["folioforge"]["raw_unicode_scalar_count"] - comparison["oracle"]["raw_unicode_scalar_count"]
        if delta < 0:
            reports.append(report)
    if len(reports) != 26:
        raise ValueError("the pinned C2 reports did not resolve to 26 FolioForge-missing cases")
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
        raise ValueError("a pinned Oracle-extra case did not resolve uniquely")
    return {digest: paths[0] for digest, paths in found.items()}


def run(args: argparse.Namespace) -> int:
    reports = negative_delta_reports(args.report_dir)
    if args.only:
        requested = set(args.only)
        available = {report["input_id"] for report in reports}
        if requested - available:
            raise ValueError("requested ID is not a FolioForge-missing C2 case")
        reports = [report for report in reports if report["input_id"] in requested]
    sources = resolve_sources(reports, args.book_root)

    books = []
    for index, report in enumerate(reports, 1):
        print(f"[{index}/{len(reports)}] inventorying Oracle-extra spans for {report['input_id']}", flush=True)
        books.append(analyze_book(report, sources[report["source_sha256_audit_only"]], args.folio))

    output = {
        "schema_version": 2,
        "scope": "content-free source-role inventory for Oracle-longer hunk spans split by Oracle event intersections; private strings were matched in memory only",
        "book_count": len(books),
        "oracle_extra_span_hunk_count": sum(book["oracle_extra_span_hunk_count"] for book in books),
        "oracle_extra_span_scalar_delta_sum": sum(book["oracle_extra_span_scalar_delta_sum"] for book in books),
        "query_segment_count": sum(book["query_segment_count"] for book in books),
        "matched_query_segment_count": sum(book["matched_query_segment_count"] for book in books),
        "exact_source_candidate_row_count": sum(
            book["exact_source_candidate_row_count"] for book in books
        ),
        "exact_source_candidate_segment_count": sum(
            book["exact_source_candidate_segment_count"] for book in books
        ),
        "source_query_classification_counts": dict(
            sorted(
                sum(
                    (Counter(book["source_query_classification_counts"]) for book in books),
                    Counter(),
                ).items()
            )
        ),
        "toolchain": {
            "platform": platform.platform(),
            "folioforge_version": command_version([str(args.folio), "--version"]),
            "folioforge_binary_sha256_audit_only": file_sha256(args.folio),
            "calibre_version": command_version(["calibre-debug", "--version"]),
            "kfx_input_version": args.kfx_input_version,
            "kfx_input_plugin_archive_sha256": args.kfx_input_plugin_sha256,
            "diff_algorithm": {
                "name": "token-anchored difflib.SequenceMatcher with bounded Unicode-scalar gap refinement",
                "token_pattern": r"\s+|\w+|[^\w\s]",
                "sequence_matcher_autojunk": True,
                "character_gap_max_scalar_length": 8_000,
                "character_gap_max_scalar_product": 2_000_000,
                "primary_comparison": "raw Unicode scalar sequence",
                "secondary_diagnostics": [
                    "CRLF/CR to LF",
                    "Unicode NFC",
                    "whitespace collapse",
                    "control/format replacement",
                ],
            },
        },
        "books": books,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("wrote content-free Oracle-extra source-role report", flush=True)
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", type=Path, required=True)
    result.add_argument("--folio", type=Path, required=True)
    result.add_argument("--report-dir", type=Path, default=Path("tests-private/kfx-corpus/c2-r"))
    result.add_argument(
        "--output",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r/oracle-extra-inventory-v2.json"),
    )
    result.add_argument("--only", action="append", default=[])
    result.add_argument("--kfx-input-version", default="2.34.2")
    result.add_argument(
        "--kfx-input-plugin-sha256",
        default="338809c18e5f9bb721dc3570a64cf6f7add4a1711c8183e6179e90fcbadf3c1d",
    )
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"C2 Oracle-extra source-string audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
