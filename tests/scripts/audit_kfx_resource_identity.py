#!/usr/bin/env python3
"""Create a path-free, reproducible identity audit for a local KFX corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path


def digest_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def audit_file(binary: Path, path: Path) -> dict:
    result = subprocess.run(
        [str(binary), "inspect", str(path), "--kfx-resource-audit"],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise RuntimeError(f"KFX audit failed for one local input (exit {result.returncode})")
    return json.loads(result.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "corpus",
        nargs="?",
        type=Path,
        default=Path(os.environ.get("FOLIOFORGE_KFX_CORPUS", "private-corpus")),
        help="external directory containing DRM-free .kfx test inputs",
    )
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path("target/debug/folio"),
        help="path to the FolioForge CLI binary",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("local-audit/kfx-resource-identity.json"),
        help="path-free JSON report destination",
    )
    args = parser.parse_args()

    if not args.corpus.is_dir():
        parser.error("KFX corpus directory does not exist")
    if not args.binary.is_file():
        parser.error("FolioForge CLI binary does not exist; build folio-cli first")

    paths = sorted(
        (path for path in args.corpus.rglob("*") if path.is_file() and path.suffix.lower() == ".kfx"),
        key=lambda path: path.as_posix(),
    )
    audited = []
    for path in paths:
        audited.append((digest_file(path), audit_file(args.binary, path)))

    all_records = [record for _, report in audited for record in report["external_resources"]]
    exact_count = sum(
        record["identity_classification"] == "ResolvedByExactIdentity" for record in all_records
    )
    dimension_only_count = sum(
        record["prior_dimension_only_match_audit_only"] for record in all_records
    )
    production_unresolved = [
        record for record in all_records if record["production_binding_status"] != "Exact"
    ]
    legacy_85_by_hash: dict[str, list[dict]] = {}
    all_by_hash = dict(audited)
    for file_hash, report in audited:
        legacy_85_by_hash[file_hash] = [
            record
            for record in report["external_resources"]
            if not record["candidate_raw_media_legacy_unadjusted_audit_only"]
            and not record["prior_dimension_only_match_audit_only"]
        ]

    affected = [
        (file_hash, all_by_hash[file_hash], legacy_records)
        for file_hash, legacy_records in legacy_85_by_hash.items()
        if legacy_records
    ]
    affected.sort(key=lambda item: item[0])
    books = []
    for index, (file_hash, report, legacy_records) in enumerate(affected, start=1):
        legacy_keys = {
            (
                record["fragment_key"]["container_origin"],
                record["fragment_key"]["native_entity_id"],
            )
            for record in legacy_records
        }
        records = []
        for record in report["external_resources"]:
            key = (
                record["fragment_key"]["container_origin"],
                record["fragment_key"]["native_entity_id"],
            )
            records.append({**record, "part_of_legacy_85": key in legacy_keys})
        books.append(
            {
                "book_id": f"KFX-RI-{index:03}",
                "file_sha256": file_hash,
                "container_count": len(report["containers"]),
                "entity_count": sum(
                    container["entity_count"] for container in report["containers"]
                ),
                "external_resource_count": report["summary"]["external_resource_count"],
                "raw_media_count": report["summary"]["raw_media_count"],
                "production_unresolved_count": report["summary"][
                    "production_unresolved_bindings"
                ],
                "visible_placement_count": sum(
                    record["visible_placement_count"] for record in report["external_resources"]
                ),
                "summary": report["summary"],
                "containers": report["containers"],
                "legacy_85_records": legacy_records,
                "external_resources": records,
                "raw_media": report["raw_media"],
                "composite_fragments": report["composite_fragments"],
            }
        )

    legacy_85_records = [record for records in legacy_85_by_hash.values() for record in records]
    legacy_85_resolved = sum(
        record["production_binding_status"] == "Exact"
        and record["identity_classification"] == "ResolvedByExactIdentity"
        for record in legacy_85_records
    )
    legacy_85_unsupported = sum(
        record["production_diagnostic_code"] in {"KFX-R004", "KFX-R005"}
        for record in legacy_85_records
    )
    legacy_85_missing_backing = sum(
        record["production_diagnostic_code"] == "KFX-R002"
        for record in legacy_85_records
    )
    report = {
        "schema_version": 1,
        "policy": {
            "production_identity": "exact $165 location == decoded $417 fid only",
            "dimensions_order_hash_and_proximity": "audit-only; never a production binding key",
            "unproven_associations": "remain StillUnknown and are not materialized as image placements",
            "sha256": "audit-only fingerprint; never used to bind a resource",
            "source_paths_and_book_metadata": "not included",
        },
        "corpus_summary": {
            "audited_books": len(audited),
            "external_resource_records": len(all_records),
            "resolved_by_exact_identity": exact_count,
            "production_exact_bindings": sum(
                record["production_binding_status"] == "Exact" for record in all_records
            ),
            "production_unresolved_bindings": len(production_unresolved),
            "production_unresolved_visible_placement_positions": sum(
                record["visible_placement_count"] for record in production_unresolved
            ),
            "strict_unresolved": len(production_unresolved),
            "body_resource_reference_occurrences": sum(
                record["reference_count"] for record in all_records
            ),
            "visible_placement_positions": sum(
                record["visible_placement_count"] for record in all_records
            ),
            "standard_ion_sid_lookup_reference_occurrences_audit_only": sum(
                record["reference_count_standard_ion_audit_only"]
                for record in all_records
            ),
            "standard_ion_sid_lookup_visible_positions_audit_only": sum(
                record["visible_placement_count_standard_ion_audit_only"]
                for record in all_records
            ),
            "previous_dimension_only_matches_audit_only": dimension_only_count,
            "pre_correction_exact_identity_matches_audit_only": sum(
                bool(record["candidate_raw_media_legacy_unadjusted_audit_only"])
                for record in all_records
            ),
            "recovered_by_corrected_symbol_ids": sum(
                record["production_binding_status"] == "Exact"
                and not record["candidate_raw_media_legacy_unadjusted_audit_only"]
                for record in all_records
            ),
            "legacy_unadjusted_symbol_offset_matches_audit_only": sum(
                bool(record["candidate_raw_media_legacy_unadjusted_audit_only"])
                and not record["candidate_raw_media"]
                for record in all_records
            ),
            "legacy_unresolved_85": sum(map(len, legacy_85_by_hash.values())),
            "legacy_85_referenced_occurrences": sum(
                record["reference_count"]
                for records in legacy_85_by_hash.values()
                for record in records
            ),
            "legacy_85_visible_placement_positions": sum(
                record["visible_placement_count"]
                for records in legacy_85_by_hash.values()
                for record in records
            ),
            "legacy_85_affected_books": len(affected),
            "strict_unresolved_referenced_occurrences": sum(
                record["reference_count"] for record in production_unresolved
            ),
            "strict_unresolved_visible_placement_positions": sum(
                record["visible_placement_count"] for record in production_unresolved
            ),
        },
        "classification": {
            "legacy_85_resolved_by_exact_identity": legacy_85_resolved,
            "legacy_85_resolved_by_supported_composite_rule": 0,
            "legacy_85_confirmed_unsupported_variant": legacy_85_unsupported,
            "legacy_85_confirmed_missing_source_backing": legacy_85_missing_backing,
            "legacy_85_still_unknown": len(legacy_85_records)
            - legacy_85_resolved
            - legacy_85_unsupported
            - legacy_85_missing_backing,
            "note": "Historical membership is reconstructed from the pre-correction symbol-ID view. No unsupported/missing-backing label is asserted without evidence.",
        },
        "affected_books": books,
    }

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "audited_books": len(audited),
                "legacy_85": report["corpus_summary"]["legacy_unresolved_85"],
                "affected_books": len(affected),
                "strict_unresolved": len(production_unresolved),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"resource audit failed: {error}", file=sys.stderr)
        raise SystemExit(1)
