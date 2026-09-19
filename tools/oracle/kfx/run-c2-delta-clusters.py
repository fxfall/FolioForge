#!/usr/bin/env python3
"""Create content-free direction/delta clusters for the 36 C2 count mismatches."""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter
from pathlib import Path
from typing import Any, Optional


FEATURE_SIGNALS = (
    "has_stories",
    "story_definitions",
    "story_references",
    "stories_traversed_unique",
    "stories_emitted_more_than_once",
    "story_definitions_not_emitted",
    "has_conditional_content",
    "has_illustrated_layout",
    "has_footnotes",
    "has_non_image_render_inline",
    "has_ruby",
    "has_mathml",
)


def safe_counts(value: Any) -> dict[str, int]:
    if not isinstance(value, dict):
        return {}
    return {str(key): int(count) for key, count in value.items() if isinstance(count, int)}


def summarize_count_mismatch(report: dict[str, Any]) -> Optional[dict[str, Any]]:
    comparison = report.get("comparison", {})
    oracle = comparison.get("oracle", {})
    folio = comparison.get("folioforge", {})
    oracle_count = oracle.get("raw_unicode_scalar_count")
    folio_count = folio.get("raw_unicode_scalar_count")
    if not isinstance(oracle_count, int) or not isinstance(folio_count, int):
        raise ValueError("C2 diff report is missing scalar counts")
    delta = folio_count - oracle_count
    if delta == 0:
        return None

    hunks = comparison.get("hunks", [])
    operations: Counter[str] = Counter()
    signatures: Counter[str] = Counter()
    oracle_categories: Counter[str] = Counter()
    folio_categories: Counter[str] = Counter()
    inserted_scalars = 0
    deleted_scalars = 0
    replaced_oracle_scalars = 0
    replaced_folio_scalars = 0
    source_event_count = 0
    source_value_kinds: Counter[str] = Counter()
    source_leaf_field_ids: Counter[str] = Counter()
    sections: set[str] = set()
    stories: set[str] = set()

    for hunk in hunks:
        kind = hunk.get("kind", "unknown")
        operations[kind] += 1
        signatures[hunk.get("signature", "unknown")] += 1
        oracle_len = int(hunk.get("oracle_len", 0))
        folio_len = int(hunk.get("folio_len", 0))
        if kind == "insert":
            inserted_scalars += folio_len
        elif kind == "delete":
            deleted_scalars += oracle_len
        elif kind == "replace":
            replaced_oracle_scalars += oracle_len
            replaced_folio_scalars += folio_len
        oracle_categories.update(safe_counts(hunk.get("oracle_classes", {}).get("counts")))
        folio_categories.update(safe_counts(hunk.get("folio_classes", {}).get("counts")))
        provenance = hunk.get("folio_source_provenance", {})
        events = provenance.get("events", []) if isinstance(provenance, dict) else []
        source_event_count += len(events)
        for event in events:
            value_kind = event.get("source_value_kind")
            if isinstance(value_kind, str):
                source_value_kinds[value_kind] += 1
            source_path = event.get("source_path")
            if isinstance(source_path, str):
                field_ids = re.findall(r"\$(\d+)", source_path)
                if field_ids:
                    source_leaf_field_ids["$" + field_ids[-1]] += 1
        sections.update(
            section for section in provenance.get("section_ids", []) if isinstance(section, str)
        )
        stories.update(
            story for story in provenance.get("story_ids", []) if isinstance(story, str)
        )

    if inserted_scalars - deleted_scalars + replaced_folio_scalars - replaced_oracle_scalars != delta:
        raise ValueError("hunk scalar deltas do not reconcile to the book-level delta")

    features = folio.get("source_features", {})
    feature_signals = {
        key: features.get(key)
        for key in FEATURE_SIGNALS
        if isinstance(features, dict) and key in features
    }
    unknown_feature_flags = sorted(
        value
        for value in features.get("undecoded_features", [])
        if isinstance(value, str)
    ) if isinstance(features, dict) else []

    return {
        "input_id": report.get("input_id"),
        "source_sha256_audit_only": report.get("source_sha256_audit_only"),
        "oracle_scalar_count": oracle_count,
        "folioforge_scalar_count": folio_count,
        "delta_folio_minus_oracle": delta,
        "absolute_delta": abs(delta),
        "direction": "folioforge_extra" if delta > 0 else "folioforge_missing",
        "root_cause_status": "UnknownRootCause",
        "hunk_count": len(hunks),
        "hunk_operation_counts": dict(sorted(operations.items())),
        "hunk_signature_counts": dict(sorted(signatures.items())),
        "oracle_hunk_scalar_category_counts": dict(sorted(oracle_categories.items())),
        "folioforge_hunk_scalar_category_counts": dict(sorted(folio_categories.items())),
        "inserted_folio_scalars": inserted_scalars,
        "deleted_oracle_scalars": deleted_scalars,
        "replaced_oracle_scalars": replaced_oracle_scalars,
        "replaced_folio_scalars": replaced_folio_scalars,
        "source_provenance_event_count": source_event_count,
        "hunk_source_value_kind_counts": dict(sorted(source_value_kinds.items())),
        "hunk_source_leaf_field_id_counts": dict(sorted(source_leaf_field_ids.items())),
        "proven_sections": len(sections),
        "proven_stories": len(stories),
        "feature_signals": feature_signals,
        "undecoded_feature_flags": unknown_feature_flags,
    }


def build_clusters(report_dir: Path) -> dict[str, Any]:
    books = []
    for path in sorted(report_dir.glob("KFX-C*/text-diff.json")):
        report = json.loads(path.read_text(encoding="utf-8"))
        summary = summarize_count_mismatch(report)
        if summary is not None:
            books.append(summary)
    if len(books) != 36:
        raise ValueError("the pinned C2 diff reports did not resolve to 36 count mismatches")

    grouped: dict[str, list[dict[str, Any]]] = {
        "folioforge_extra": [],
        "folioforge_missing": [],
    }
    for book in books:
        grouped[book["direction"]].append(book)
    for rows in grouped.values():
        rows.sort(key=lambda book: (book["absolute_delta"], book["input_id"]))

    absolute_delta_groups = Counter(book["absolute_delta"] for book in books)
    return {
        "schema_version": 1,
        "scope": "direction and scalar-delta signatures only; root causes remain UnknownRootCause",
        "book_count": len(books),
        "net_delta_folio_minus_oracle": sum(book["delta_folio_minus_oracle"] for book in books),
        "sum_absolute_delta": sum(book["absolute_delta"] for book in books),
        "absolute_delta_book_counts": {
            str(delta): count for delta, count in sorted(absolute_delta_groups.items())
        },
        "direction_groups": {
            direction: {
                "book_count": len(rows),
                "books": rows,
            }
            for direction, rows in grouped.items()
        },
    }


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        "--report-dir",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r"),
    )
    result.add_argument(
        "--output",
        type=Path,
        default=Path("tests-private/kfx-corpus/c2-r/delta-clusters.json"),
    )
    return result


if __name__ == "__main__":
    try:
        args = parser().parse_args()
        report = build_clusters(args.report_dir)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(
            f"wrote {report['book_count']} direction/delta clusters; "
            f"net delta {report['net_delta_folio_minus_oracle']}",
            flush=True,
        )
    except (OSError, ValueError) as error:
        raise SystemExit(f"C2 delta clustering failed: {error}") from error
