#!/usr/bin/env python3
"""Create anonymous, content-free Unicode diff reports for KFX-C2 mismatches."""

from __future__ import annotations

import argparse
import bisect
import difflib
import hashlib
import json
import re
import subprocess
import sys
import unicodedata
from collections import Counter
from pathlib import Path
from typing import Any

TOKEN_PATTERN = re.compile(r"\s+|\w+|[^\w\s]", re.UNICODE)


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def raw_oracle_events(raw: dict[str, Any]) -> tuple[str, list[dict[str, Any]]]:
    rows = raw.get("data")
    if not isinstance(rows, list):
        raise ValueError("unexpected KFX Input JSON structure")
    selected = [
        (row["position"], ordinal, row["content"])
        for ordinal, row in enumerate(rows)
        if isinstance(row, dict)
        and row.get("type") == 1
        and isinstance(row.get("position"), int)
        and not isinstance(row.get("position"), bool)
        and isinstance(row.get("content"), str)
    ]
    selected.sort(key=lambda occurrence: (occurrence[0], occurrence[1]))
    events = []
    pieces = []
    offset = 0
    for position, row_order, content in selected:
        length = len(content)
        events.append(
            {
                "pid": position,
                "row_order": row_order,
                "start": offset,
                "length": length,
                "sha256_raw_audit_only": sha256_text(content),
                "content": content,
            }
        )
        pieces.append(content)
        offset += length
    return "".join(pieces), events


def native_event_stream(report: dict[str, Any]) -> tuple[str, list[dict[str, Any]]]:
    rows = report.get("events")
    if not isinstance(rows, list):
        raise ValueError("FolioForge event report has no event list")
    pieces: list[str] = []
    events: list[dict[str, Any]] = []
    offset = 0
    for event in rows:
        if not isinstance(event, dict) or not isinstance(event.get("text"), str):
            raise ValueError("private FolioForge text event is missing")
        content = event["text"]
        length = len(content)
        if event.get("text_len") != length:
            raise ValueError("FolioForge event scalar length did not reconcile")
        if event.get("text_hash_raw_audit_only") != sha256_text(content):
            raise ValueError("FolioForge event raw digest did not reconcile")
        events.append({**event, "start": offset, "end": offset + length})
        pieces.append(content)
        offset += length
    stream = "".join(pieces)
    summary = report.get("stream", {})
    if summary.get("raw_unicode_scalar_count") != len(stream):
        raise ValueError("FolioForge raw event lengths did not reconcile")
    if summary.get("raw_sha256_audit_only") != sha256_text(stream):
        raise ValueError("FolioForge raw event stream digest did not reconcile")
    return stream, events


def category_name(char: str) -> str:
    category = unicodedata.category(char)
    if char.isspace():
        return "space"
    if category.startswith("L"):
        return "letter"
    if category.startswith("M"):
        return "mark"
    if category.startswith("N"):
        return "number"
    if category.startswith("P"):
        return "punctuation"
    if category.startswith("S"):
        return "symbol"
    if category.startswith("C"):
        return "control_or_format"
    return "separator_or_other"


def scalar_categories(value: str) -> dict[str, Any]:
    categories = Counter(category_name(char) for char in value)
    special = Counter(char for char in value if unicodedata.category(char).startswith("C"))
    codepoints = Counter(char for char in value if char.isspace() and char != " ")
    inventory = []
    for char, count in (special + codepoints).most_common(16):
        inventory.append(
            {
                "codepoint": "U+{:04X}".format(ord(char)),
                "unicode_name": unicodedata.name(char, "UNNAMED"),
                "category": unicodedata.category(char),
                "count": count,
            }
        )
    return {
        "counts": dict(sorted(categories.items())),
        "non_ascii_space_and_control_inventory": inventory,
    }


def normalize_line_endings(value: str) -> str:
    return value.replace("\r\n", "\n").replace("\r", "\n")


def normalize_whitespace(value: str) -> str:
    return re.sub(r"\s+", " ", value, flags=re.UNICODE).strip()


def normalize_controls(value: str) -> str:
    return "".join(
        "\uFFFD" if unicodedata.category(char) in {"Cc", "Cf"} else char
        for char in value
    )


def normalization_diagnostics(oracle: str, folio: str) -> dict[str, bool]:
    oracle_lines = normalize_line_endings(oracle)
    folio_lines = normalize_line_endings(folio)
    oracle_nfc = unicodedata.normalize("NFC", oracle)
    folio_nfc = unicodedata.normalize("NFC", folio)
    return {
        "raw_equal": oracle == folio,
        "line_endings_normalized_equal": oracle_lines == folio_lines,
        "nfc_only_equal": oracle_nfc == folio_nfc,
        "line_endings_then_nfc_equal": unicodedata.normalize("NFC", oracle_lines)
        == unicodedata.normalize("NFC", folio_lines),
        "whitespace_collapsed_equal": normalize_whitespace(oracle)
        == normalize_whitespace(folio),
        "control_and_format_replaced_equal": normalize_controls(oracle)
        == normalize_controls(folio),
    }


def common_prefix_length(left: str, right: str) -> int:
    limit = min(len(left), len(right))
    index = 0
    while index < limit and left[index] == right[index]:
        index += 1
    return index


def common_suffix_length(left: str, right: str, prefix: int) -> int:
    limit = min(len(left), len(right)) - prefix
    count = 0
    while count < limit and left[len(left) - 1 - count] == right[len(right) - 1 - count]:
        count += 1
    return count


def event_summary(event: dict[str, Any]) -> dict[str, Any]:
    fields = (
        "trace_id",
        "section_id",
        "story_id",
        "source_fragment",
        "source_fragment_order",
        "eid",
        "source_path",
        "source_value_kind",
        "source_position",
        "source_position_kind",
        "context",
        "context_basis",
        "text_len",
        "start",
        "end",
    )
    return {field: event.get(field) for field in fields}


def folio_provenance(
    start: int, end: int, events: list[dict[str, Any]], starts: list[int]
) -> dict[str, Any]:
    if not events:
        return {"event_count": 0, "events": [], "truncated": False}
    if end > start:
        first = max(0, bisect.bisect_right(starts, start) - 1)
        last = max(first, bisect.bisect_left(starts, end))
        overlapping = [
            event
            for event in events[first : last + 1]
            if event["end"] > start and event["start"] < end
        ]
    else:
        index = max(0, bisect.bisect_right(starts, start) - 1)
        overlapping = [events[index]]
    if not overlapping:
        index = min(max(0, bisect.bisect_right(starts, start) - 1), len(events) - 1)
        overlapping = [events[index]]
    limit = 24
    selected = overlapping[:limit]
    return {
        "event_count": len(overlapping),
        "events": [event_summary(event) for event in selected],
        "truncated": len(overlapping) > limit,
        "source_fragments": list(dict.fromkeys(e["source_fragment"] for e in overlapping))[:24],
        "section_ids": list(dict.fromkeys(e.get("section_id") for e in overlapping)),
        "story_ids": list(dict.fromkeys(e.get("story_id") for e in overlapping)),
    }


def hunk_signature(kind: str, oracle: str, folio: str) -> str:
    if kind == "insert":
        return "folio_extra"
    if kind == "delete":
        return "folio_missing"
    if kind != "replace":
        return kind
    if len(oracle) == len(folio):
        if oracle == folio:
            return "equal_length_identical"
        if oracle and all(char.isspace() for char in oracle) and not all(
            char.isspace() for char in folio
        ):
            return "equal_length_oracle_whitespace_replaced"
        if all(char.isspace() for char in oracle + folio):
            if any(char in "\u00A0\u202F" for char in oracle + folio):
                return "equal_length_non_ascii_space_candidate"
            return "equal_length_whitespace_substitution"
        if any(char in "\u00A0\u202F" for char in oracle + folio):
            return "equal_length_non_ascii_space_candidate"
        if any(unicodedata.category(char) == "Cf" for char in oracle + folio):
            return "equal_length_format_character_candidate"
        if all(unicodedata.category(char).startswith("P") for char in oracle + folio):
            return "equal_length_punctuation_substitution"
        return "equal_length_other_substitution"
    return "replace_length_changed"


def tokenize_with_offsets(value: str) -> tuple[list[str], list[int], list[int]]:
    tokens = []
    starts = []
    ends = []
    for match in TOKEN_PATTERN.finditer(value):
        tokens.append(match.group())
        starts.append(match.start())
        ends.append(match.end())
    return tokens, starts, ends


def token_boundary(starts: list[int], total_length: int, index: int) -> int:
    return starts[index] if index < len(starts) else total_length


def token_range_end(
    starts: list[int], ends: list[int], total_length: int, start: int, end: int
) -> int:
    if start == end:
        return token_boundary(starts, total_length, start)
    return ends[end - 1]


def refine_character_gap(
    oracle: str,
    folio: str,
    oracle_start: int,
    folio_start: int,
    output: list[tuple[str, int, int, int, int]],
) -> None:
    if not oracle:
        output.append(("insert", oracle_start, oracle_start, folio_start, folio_start + len(folio)))
        return
    if not folio:
        output.append(("delete", oracle_start, oracle_start + len(oracle), folio_start, folio_start))
        return
    if max(len(oracle), len(folio)) > 8_000 or len(oracle) * len(folio) > 2_000_000:
        output.append(("replace", oracle_start, oracle_start + len(oracle), folio_start, folio_start + len(folio)))
        return
    matcher = difflib.SequenceMatcher(a=oracle, b=folio, autojunk=True)
    for kind, a_start, a_end, b_start, b_end in matcher.get_opcodes():
        output.append(
            (
                kind,
                oracle_start + a_start,
                oracle_start + a_end,
                folio_start + b_start,
                folio_start + b_end,
            )
        )


def anchored_unicode_opcodes(
    oracle: str, folio: str
) -> tuple[list[tuple[str, int, int, int, int]], int]:
    prefix = common_prefix_length(oracle, folio)
    suffix = common_suffix_length(oracle, folio, prefix)
    oracle_middle = oracle[prefix : len(oracle) - suffix if suffix else len(oracle)]
    folio_middle = folio[prefix : len(folio) - suffix if suffix else len(folio)]
    oracle_tokens, oracle_starts, oracle_ends = tokenize_with_offsets(oracle_middle)
    folio_tokens, folio_starts, folio_ends = tokenize_with_offsets(folio_middle)
    matcher = difflib.SequenceMatcher(a=oracle_tokens, b=folio_tokens, autojunk=True)
    operations: list[tuple[str, int, int, int, int]] = []
    if prefix:
        operations.append(("equal", 0, prefix, 0, prefix))
    oracle_cursor = 0
    folio_cursor = 0
    refined_gap_count = 0
    for kind, a_start, a_end, b_start, b_end in matcher.get_opcodes():
        a_char_start = token_boundary(oracle_starts, len(oracle_middle), a_start)
        a_char_end = token_range_end(
            oracle_starts, oracle_ends, len(oracle_middle), a_start, a_end
        )
        b_char_start = token_boundary(folio_starts, len(folio_middle), b_start)
        b_char_end = token_range_end(
            folio_starts, folio_ends, len(folio_middle), b_start, b_end
        )
        if a_char_start < oracle_cursor or b_char_start < folio_cursor:
            raise ValueError("token diff generated overlapping character ranges")
        if a_char_start > oracle_cursor or b_char_start > folio_cursor:
            refine_character_gap(
                oracle_middle[oracle_cursor:a_char_start],
                folio_middle[folio_cursor:b_char_start],
                prefix + oracle_cursor,
                prefix + folio_cursor,
                operations,
            )
            refined_gap_count += 1
        if kind == "equal":
            if oracle_middle[a_char_start:a_char_end] != folio_middle[b_char_start:b_char_end]:
                raise ValueError("equal token anchor did not match source Unicode text")
            operations.append(
                (
                    "equal",
                    prefix + a_char_start,
                    prefix + a_char_end,
                    prefix + b_char_start,
                    prefix + b_char_end,
                )
            )
        else:
            refine_character_gap(
                oracle_middle[a_char_start:a_char_end],
                folio_middle[b_char_start:b_char_end],
                prefix + a_char_start,
                prefix + b_char_start,
                operations,
            )
            refined_gap_count += 1
        oracle_cursor = a_char_end
        folio_cursor = b_char_end
    if oracle_cursor < len(oracle_middle) or folio_cursor < len(folio_middle):
        refine_character_gap(
            oracle_middle[oracle_cursor:],
            folio_middle[folio_cursor:],
            prefix + oracle_cursor,
            prefix + folio_cursor,
            operations,
        )
        refined_gap_count += 1
    if suffix:
        operations.append(
            (
                "equal",
                len(oracle) - suffix,
                len(oracle),
                len(folio) - suffix,
                len(folio),
            )
        )
    return operations, refined_gap_count


def make_hunks(
    oracle: str, folio: str, events: list[dict[str, Any]]
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    operations, refined_gap_count = anchored_unicode_opcodes(oracle, folio)
    changed = []
    starts = [event["start"] for event in events]
    changed_hunks = 0
    changed_ranges: list[tuple[int, int, int, int, str]] = []
    for kind, oracle_start, oracle_end, folio_start, folio_end in operations:
        oracle_piece = oracle[oracle_start:oracle_end]
        folio_piece = folio[folio_start:folio_end]
        operation = {
            "kind": kind,
            "oracle_start": oracle_start,
            "oracle_len": oracle_end - oracle_start,
            "folio_start": folio_start,
            "folio_len": folio_end - folio_start,
        }
        if kind == "equal":
            continue
        changed_hunks += 1
        changed_ranges.append(
            (oracle_start, oracle_end, folio_start, folio_end, kind)
        )
        changed.append(
            {
                **operation,
                "signature": hunk_signature(kind, oracle_piece, folio_piece),
                "oracle_classes": scalar_categories(oracle_piece),
                "folio_classes": scalar_categories(folio_piece),
                "folio_source_provenance": folio_provenance(
                    folio_start, folio_end, events, starts
                ),
            }
        )

    equal_length_only = bool(changed_ranges) and all(
        (oracle_end - oracle_start) == (folio_end - folio_start)
        for oracle_start, oracle_end, folio_start, folio_end, _ in changed_ranges
    )
    prefix = common_prefix_length(oracle, folio)
    suffix = common_suffix_length(oracle, folio, prefix)
    summary = {
        "diff_algorithm": "token-anchored difflib.SequenceMatcher with bounded Unicode-scalar gap refinement",
        "operation_count_including_equal": len(operations),
        "refined_gap_count": refined_gap_count,
        "hunk_count": changed_hunks,
        "equal_length_replacements_only": equal_length_only,
        "changed_scalar_delta_folio_minus_oracle": len(folio) - len(oracle),
        "first_divergent_unicode_index": None if not changed_ranges else prefix,
        "common_prefix_length": prefix,
        "common_suffix_length": suffix,
        "hunk_signatures": dict(sorted(Counter(h["signature"] for h in changed).items())),
    }
    return changed, summary


def book_signatures(
    oracle: str,
    folio: str,
    diff_summary: dict[str, Any],
    diagnostics: dict[str, bool],
    hunks: list[dict[str, Any]],
) -> list[str]:
    signatures = []
    delta = len(folio) - len(oracle)
    if diagnostics["nfc_only_equal"] and oracle != folio:
        signatures.append("unicode-normalization-difference")
    elif diagnostics["line_endings_normalized_equal"] and oracle != folio:
        signatures.append("line-ending-difference")
    elif delta == 0 and diff_summary["equal_length_replacements_only"]:
        signatures.append("same-length-only")
    elif delta > 0:
        signatures.append("folio-longer")
    elif delta < 0:
        signatures.append("oracle-longer")
    if diagnostics["whitespace_collapsed_equal"] and oracle != folio:
        signatures.append("whitespace-normalized-equal")
    if diagnostics["control_and_format_replaced_equal"] and oracle != folio:
        signatures.append("control-or-format-normalized-equal")
    section_ids = {
        section
        for hunk in hunks
        for section in hunk["folio_source_provenance"].get("section_ids", [])
        if section is not None
    }
    if len(section_ids) == 1:
        signatures.append("localized-single-section")
    elif len(section_ids) > 1:
        signatures.append("multi-section")
    if not signatures:
        signatures.append("unknown-diff-shape")
    return signatures


def run(args: argparse.Namespace) -> int:
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    comparison = json.loads(args.comparison.read_text(encoding="utf-8"))
    entries = manifest.get("books")
    if not isinstance(entries, list) or len(entries) != 82:
        raise ValueError("pinned anonymous corpus must contain 82 books")
    comparison_books = comparison.get("books")
    if not isinstance(comparison_books, list) or len(comparison_books) != 82:
        raise ValueError("existing C2 comparison must contain 82 books")
    by_id = {book["id"]: book for book in entries}
    mismatches = []
    for result in comparison_books:
        parity = result.get("stage_parity", {}).get("calibre_vs_native", {})
        if not parity.get("normalized_text_hash_equal") or not parity.get(
            "unicode_scalar_count_equal"
        ):
            mismatches.append(result["input_id"])
    if len(mismatches) != 56 or any(book_id not in by_id for book_id in mismatches):
        raise ValueError("existing C2 mismatch set did not reconcile to 56 anonymous inputs")
    if args.only:
        unknown = set(args.only) - set(mismatches)
        if unknown:
            raise ValueError("requested anonymous ID is not in the C2 mismatch set")
        mismatches = [book_id for book_id in mismatches if book_id in set(args.only)]

    source_by_hash: dict[str, list[Path]] = {book["source_sha256_audit_only"]: [] for book in entries}
    for path in args.book_root.iterdir():
        if path.is_file() and not path.is_symlink() and path.suffix.lower() == ".kfx":
            digest = file_sha256(path)
            if digest in source_by_hash:
                source_by_hash[digest].append(path)
    if any(len(paths) != 1 for paths in source_by_hash.values()):
        raise ValueError("a pinned corpus input did not resolve uniquely")

    args.output.mkdir(parents=True, exist_ok=True)
    results = []
    for book_id in mismatches:
        item = by_id[book_id]
        book_path = source_by_hash[item["source_sha256_audit_only"]][0]
        fidelity_path = args.fidelity_report_dir / f"{book_id}.json"
        fidelity = json.loads(fidelity_path.read_text(encoding="utf-8"))
        if fidelity.get("input_sha256_audit_only") != item["source_sha256_audit_only"]:
            raise ValueError("paired content-free FolioForge report did not reconcile")

        calibre_path = args.temp_dir / f"{book_id}.calibre.json"
        try:
            result = subprocess.run(
                [
                    "calibre-debug",
                    "-r",
                    "KFX Input",
                    "--",
                    "--json-content",
                    str(book_path),
                    str(calibre_path),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                check=False,
                timeout=300,
            )
            if result.returncode != 0 or not calibre_path.is_file():
                raise RuntimeError("KFX Input text oracle failed")
            calibre_raw = json.loads(calibre_path.read_text(encoding="utf-8"))
            oracle_text, oracle_events = raw_oracle_events(calibre_raw)
            del calibre_raw
        finally:
            calibre_path.unlink(missing_ok=True)

        folio_result = subprocess.run(
            [
                str(args.folio),
                "inspect",
                str(book_path),
                "--kfx-text-event-audit",
                "--include-private-text",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            timeout=600,
        )
        if folio_result.returncode != 0:
            raise RuntimeError("FolioForge native text event audit failed")
        folio_event_report = json.loads(folio_result.stdout)
        folio_features = folio_event_report.get("source_features", {})
        folio_text, folio_events = native_event_stream(folio_event_report)
        del folio_result

        source_text_report = fidelity.get("text", {})
        folio_normalized = folio_event_report.get("stream", {})
        if (
            folio_normalized.get("normalized_unicode_scalar_count")
            != source_text_report.get("source_unicode_scalar_count")
            or folio_normalized.get("normalized_sha256_audit_only")
            != source_text_report.get("source_normalized_sha256_audit_only")
        ):
            raise ValueError("TextEvent stream did not reconcile with the pinned native audit")
        del folio_event_report

        hunks, diff_summary = make_hunks(oracle_text, folio_text, folio_events)
        diagnostics = normalization_diagnostics(oracle_text, folio_text)
        report = {
            "schema_version": 1,
            "input_id": book_id,
            "source_sha256_audit_only": item["source_sha256_audit_only"],
            "comparison": {
                "oracle": {
                    "name": "Calibre KFX Input",
                    "text_event_count": len(oracle_events),
                    "raw_unicode_scalar_count": len(oracle_text),
                    "raw_sha256_audit_only": sha256_text(oracle_text),
                    "ordering": "type=1 rows sorted by generated PID then source row order",
                },
                "folioforge": {
                    "text_event_count": len(folio_events),
                    "raw_unicode_scalar_count": len(folio_text),
                    "raw_sha256_audit_only": sha256_text(folio_text),
                    "source_features": folio_features,
                    "ordering": "native KFX content-fragment and Ion tree traversal",
                },
                "primary_comparison": "raw Unicode scalar sequence; no NFC/whitespace normalization",
                "normalization_diagnostics": diagnostics,
                "diff": diff_summary,
                "book_signatures": book_signatures(
                    oracle_text, folio_text, diff_summary, diagnostics, hunks
                ),
                "hunks": hunks,
            },
        }
        report_path = args.output / book_id / "text-diff.json"
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        results.append(
            {
                "input_id": book_id,
                "oracle_scalars": len(oracle_text),
                "folio_scalars": len(folio_text),
                "hunk_count": diff_summary["hunk_count"],
                "book_signatures": report["comparison"]["book_signatures"],
            }
        )
        print(f"Created content-free text diff for {book_id}.", flush=True)

    args.output.joinpath("summary.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "corpus_input_count": len(entries),
                "mismatch_count": len(results),
                "full_c2_mismatch_count": 56,
                "unknown_diff_shape_count": sum(
                    "unknown-diff-shape" in row["book_signatures"] for row in results
                ),
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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--book-root", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--comparison", required=True, type=Path)
    parser.add_argument("--fidelity-report-dir", required=True, type=Path)
    parser.add_argument("--folio", required=True, type=Path)
    parser.add_argument("--temp-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--only", action="append", default=[])
    args = parser.parse_args()
    args.temp_dir.mkdir(parents=True, exist_ok=True)
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
        print(
            "C2-R text diff failed ({}); no source text or filename was emitted.".format(
                type(error).__name__
            ),
            file=sys.stderr,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
