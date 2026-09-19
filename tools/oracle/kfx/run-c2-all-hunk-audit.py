#!/usr/bin/env python3
"""Audit every changed C2 hunk in both directions without persisting text."""

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
DIFF_SPEC = importlib.util.spec_from_file_location("folioforge_c2_diff", DIFF_PATH)
if DIFF_SPEC is None or DIFF_SPEC.loader is None:
    raise RuntimeError("unable to load the pinned C2 diff implementation")
DIFF = importlib.util.module_from_spec(DIFF_SPEC)
DIFF_SPEC.loader.exec_module(DIFF)


DIFF_ALGORITHM = {
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


def direction_for_hunk(hunk: dict[str, Any]) -> str:
    oracle_len = int(hunk["oracle_len"])
    folio_len = int(hunk["folio_len"])
    if folio_len > oracle_len:
        return "folioforge_extra"
    if oracle_len > folio_len:
        return "oracle_extra"
    if oracle_len > 0:
        return "same_length_replacement"
    return "zero_length_change"


def event_intersections(
    start: int,
    length: int,
    events: list[dict[str, Any]],
    *,
    event_start_key: str = "start",
    event_end_key: str = "end",
) -> list[dict[str, Any]]:
    end = start + length
    selected: list[dict[str, Any]] = []
    for event in events:
        event_start = int(event[event_start_key])
        event_end_value = event.get(event_end_key)
        event_end = (
            int(event_end_value)
            if event_end_value is not None
            else event_start + int(event["length"])
        )
        if length > 0:
            overlap_start = max(start, event_start)
            overlap_end = min(end, event_end)
            if overlap_start >= overlap_end:
                continue
        else:
            # A zero-width side is anchored to the event containing the
            # insertion/deletion coordinate, or to the immediately preceding
            # event when the coordinate is at a boundary.
            if not (
                event_start <= start < event_end
                or (start == event_end and event_end > event_start)
            ):
                continue
            overlap_start = start
            overlap_end = start
        summary = DIFF.event_summary(event)
        for field in (
            "pid",
            "row_order",
            "sha256_raw_audit_only",
            "text_hash_raw_audit_only",
        ):
            if field in event:
                summary[field] = event[field]
        summary.update(
            {
                "start": event_start,
                "end": event_end,
                "text_len": event_end - event_start,
                "overlap_start": overlap_start,
                "overlap_end": overlap_end,
                "overlap_length": overlap_end - overlap_start,
            }
        )
        selected.append(summary)
    return selected


def event_partition(events: list[dict[str, Any]]) -> str:
    if not events:
        return "no_event"
    if len(events) == 1:
        return "single_event"
    return "continuous_multi_event"


def side_analysis(
    start: int,
    length: int,
    value: str,
    events: list[dict[str, Any]],
) -> dict[str, Any]:
    overlapping = event_intersections(start, length, events)
    return {
        "start": start,
        "end": start + length,
        "length": length,
        "sha256_raw_audit_only": DIFF.sha256_text(value),
        "event_partition": event_partition(overlapping),
        "event_count": len(overlapping),
        "events": overlapping,
    }


def audit_hunk(
    hunk_index: int,
    hunk: dict[str, Any],
    oracle_text: str,
    folio_text: str,
    oracle_events: list[dict[str, Any]],
    folio_events: list[dict[str, Any]],
) -> dict[str, Any]:
    oracle_start = int(hunk["oracle_start"])
    oracle_len = int(hunk["oracle_len"])
    folio_start = int(hunk["folio_start"])
    folio_len = int(hunk["folio_len"])
    oracle_block = oracle_text[oracle_start : oracle_start + oracle_len]
    folio_block = folio_text[folio_start : folio_start + folio_len]
    if len(oracle_block) != oracle_len or len(folio_block) != folio_len:
        raise ValueError(f"hunk {hunk_index} exceeds a verified private stream")
    if DIFF.scalar_categories(oracle_block) != hunk["oracle_classes"]:
        raise ValueError(f"oracle scalar categories changed for hunk {hunk_index}")
    if DIFF.scalar_categories(folio_block) != hunk["folio_classes"]:
        raise ValueError(f"FolioForge scalar categories changed for hunk {hunk_index}")
    direction = direction_for_hunk(hunk)
    return {
        "hunk_index": hunk_index,
        "operation": hunk["kind"],
        "direction": direction,
        "signature": hunk["signature"],
        "oracle": side_analysis(oracle_start, oracle_len, oracle_block, oracle_events),
        "folioforge": side_analysis(folio_start, folio_len, folio_block, folio_events),
        "evidence": {
            "status": "unresolved",
            "root_cause_status": "UnknownRootCause",
            "decision": "preserve_both_sides",
            "basis": "hunk is covered by source events but reading-flow semantics are not proven",
        },
    }


def validate_pinned_streams(
    report: dict[str, Any],
    oracle_text: str,
    folio_text: str,
    oracle_events: list[dict[str, Any]],
    folio_events: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    comparison = report["comparison"]
    for side, text in (("oracle", oracle_text), ("folioforge", folio_text)):
        expected = comparison[side]
        if len(text) != expected["raw_unicode_scalar_count"]:
            raise ValueError(f"{side} stream scalar count did not match pinned C2 report")
        if DIFF.sha256_text(text) != expected["raw_sha256_audit_only"]:
            raise ValueError(f"{side} stream digest did not match pinned C2 report")
    if len(oracle_events) != comparison["oracle"]["text_event_count"]:
        raise ValueError("Calibre event count did not match pinned C2 report")
    if len(folio_events) != comparison["folioforge"]["text_event_count"]:
        raise ValueError("FolioForge event count did not match pinned C2 report")
    hunks = comparison["hunks"]
    delta = sum(int(hunk["folio_len"]) - int(hunk["oracle_len"]) for hunk in hunks)
    if delta != len(folio_text) - len(oracle_text):
        raise ValueError("hunk deltas did not reconcile to the pinned book delta")
    audited = [
        audit_hunk(index, hunk, oracle_text, folio_text, oracle_events, folio_events)
        for index, hunk in enumerate(hunks, 1)
    ]
    if len(audited) != len(hunks):
        raise ValueError("not every changed hunk received an audit record")
    return audited


def source_by_hash(book_root: Path, manifest: list[dict[str, Any]]) -> dict[str, Path]:
    wanted = {book["source_sha256_audit_only"] for book in manifest}
    found: dict[str, list[Path]] = {digest: [] for digest in wanted}
    for path in book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in found:
                found[digest].append(path)
    if any(len(paths) != 1 for paths in found.values()):
        raise ValueError("pinned corpus inputs did not resolve uniquely")
    return {digest: paths[0] for digest, paths in found.items()}


def book_report(
    report: dict[str, Any],
    source: Path,
    folio: Path,
    temp_dir: Path,
) -> dict[str, Any]:
    book_id = report["input_id"]
    calibre_path = temp_dir / f"{book_id}.calibre.json"
    try:
        result = subprocess.run(
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
            timeout=600,
        )
        if result.returncode != 0 or not calibre_path.is_file():
            raise RuntimeError(f"Calibre text oracle failed for {book_id}")
        oracle_raw = json.loads(calibre_path.read_text(encoding="utf-8"))
        oracle_text, oracle_events = DIFF.raw_oracle_events(oracle_raw)
        del oracle_raw
    finally:
        calibre_path.unlink(missing_ok=True)

    result = subprocess.run(
        [str(folio), "inspect", str(source), "--kfx-text-event-audit", "--include-private-text"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
        timeout=900,
    )
    if result.returncode != 0:
        raise RuntimeError(f"FolioForge text audit failed for {book_id}")
    folio_raw = json.loads(result.stdout)
    folio_text, folio_events = DIFF.native_event_stream(folio_raw)
    source_features = folio_raw.get("source_features", {})
    audited_hunks = validate_pinned_streams(
        report,
        oracle_text,
        folio_text,
        oracle_events,
        folio_events,
    )
    original = report["comparison"]
    direction_counts = Counter(hunk["direction"] for hunk in audited_hunks)
    hunk_delta = sum(
        hunk["folioforge"]["length"] - hunk["oracle"]["length"] for hunk in audited_hunks
    )
    book_delta = len(folio_text) - len(oracle_text)
    if hunk_delta != book_delta:
        raise ValueError(f"directional hunk deltas did not reconcile for {book_id}")
    if any(hunk["evidence"]["root_cause_status"] != "UnknownRootCause" for hunk in audited_hunks):
        raise ValueError(f"an unresolved C2 hunk was promoted for {book_id}")
    return {
        "input_id": book_id,
        "source_sha256_audit_only": report["source_sha256_audit_only"],
        "book_delta_folio_minus_oracle": book_delta,
        "oracle_scalar_count": len(oracle_text),
        "folioforge_scalar_count": len(folio_text),
        "oracle_stream_sha256_audit_only": DIFF.sha256_text(oracle_text),
        "folioforge_stream_sha256_audit_only": DIFF.sha256_text(folio_text),
        "oracle_event_count": len(oracle_events),
        "folioforge_event_count": len(folio_events),
        "direction_counts": dict(sorted(direction_counts.items())),
        "hunk_count": len(audited_hunks),
        "hunk_scalar_delta_folio_minus_oracle": hunk_delta,
        "hunk_delta_reconciles_book_delta": hunk_delta == book_delta,
        "source_features": source_features,
        "pinned_report_diff": original["diff"],
        "hunks": audited_hunks,
    }


def run(args: argparse.Namespace) -> int:
    manifest_doc = json.loads(args.manifest.read_text(encoding="utf-8"))
    manifest = manifest_doc.get("books")
    if not isinstance(manifest, list) or len(manifest) != 82:
        raise ValueError("pinned anonymous corpus must contain 82 books")
    source_paths = source_by_hash(args.book_root, manifest)
    reports: list[dict[str, Any]] = []
    for report_path in sorted(args.report_dir.glob("KFX-C*/text-diff.json")):
        report = json.loads(report_path.read_text(encoding="utf-8"))
        reports.append(report)
    if len(reports) != 56:
        raise ValueError("pinned C2-R reports must contain all 56 mismatching books")
    if args.only:
        requested = set(args.only)
        available = {report["input_id"] for report in reports}
        if requested - available:
            raise ValueError("requested input is not one of the 56 C2 mismatches")
        reports = [report for report in reports if report["input_id"] in requested]
    reports.sort(key=lambda report: report["input_id"])

    args.output.parent.mkdir(parents=True, exist_ok=True)
    books = []
    with tempfile.TemporaryDirectory(prefix="folio-c2-all-hunk-") as temporary:
        temporary_path = Path(temporary)
        for index, report in enumerate(reports, 1):
            print(f"[{index}/{len(reports)}] auditing all hunk directions for {report['input_id']}", flush=True)
            source = source_paths[report["source_sha256_audit_only"]]
            books.append(book_report(report, source, args.folio, temporary_path))

    total_hunks = sum(book["hunk_count"] for book in books)
    total_delta = sum(book["book_delta_folio_minus_oracle"] for book in books)
    total_hunk_delta = sum(book["hunk_scalar_delta_folio_minus_oracle"] for book in books)
    all_hunks = [hunk for book in books for hunk in book["hunks"]]
    output = {
        "schema_version": 2,
        "scope": "all changed hunks in the 56 C2 mismatching books; raw streams were verified in memory only",
        "book_count": len(books),
        "hunk_count": total_hunks,
        "book_net_delta_folio_minus_oracle": total_delta,
        "hunk_net_delta_folio_minus_oracle": total_hunk_delta,
        "hunk_delta_reconciles_book_delta": total_delta == total_hunk_delta,
        "direction_counts": dict(sorted(Counter(hunk["direction"] for hunk in all_hunks).items())),
        "toolchain": {
            "platform": platform.platform(),
            "folioforge_version": command_version([str(args.folio), "--version"]),
            "folioforge_binary_sha256_audit_only": file_sha256(args.folio),
            "calibre_version": command_version(["calibre-debug", "--version"]),
            "kfx_input_version": args.kfx_input_version,
            "kfx_input_plugin_archive_sha256": args.kfx_input_plugin_sha256,
            "diff_algorithm": DIFF_ALGORITHM,
        },
        "books": books,
    }
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(
        f"wrote all-hunk audit: {len(books)} books, {total_hunks} hunks, "
        f"directions reconcile={total_delta == total_hunk_delta}",
        flush=True,
    )
    return 0


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--book-root", required=True, type=Path)
    result.add_argument("--manifest", type=Path, default=Path("tests-private/kfx-corpus/corpus.json"))
    result.add_argument("--report-dir", type=Path, default=Path("tests-private/kfx-corpus/c2-r"))
    result.add_argument("--folio", required=True, type=Path)
    result.add_argument("--output", type=Path, default=Path("tests-private/kfx-corpus/c2-r/all-hunk-audit.json"))
    result.add_argument("--kfx-input-version", default="2.34.2")
    result.add_argument(
        "--kfx-input-plugin-sha256",
        default="338809c18e5f9bb721dc3570a64cf6f7add4a1711c8183e6179e90fcbadf3c1d",
    )
    result.add_argument("--only", action="append", default=[])
    return result


if __name__ == "__main__":
    try:
        raise SystemExit(run(parser().parse_args()))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        print(f"C2 all-hunk audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
