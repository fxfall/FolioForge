import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-oracle-extra-inventory.py")
SPEC = importlib.util.spec_from_file_location("run_c2_oracle_extra_inventory", SCRIPT)
INVENTORY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INVENTORY)


def oracle_event(pid, start, text):
    return {
        "pid": pid,
        "row_order": pid,
        "start": start,
        "length": len(text),
        "sha256_raw_audit_only": INVENTORY.DIFF.sha256_text(text),
    }


class C2OracleExtraInventoryTests(unittest.TestCase):
    def test_oracle_query_segments_preserve_two_event_coordinates(self):
        oracle = "0123456789"
        hunk = {"oracle_start": 2, "oracle_len": 6}
        events = [oracle_event(10, 0, "0123"), oracle_event(20, 4, "456789")]

        result = INVENTORY.oracle_query_segments(7, hunk, oracle, events)

        self.assertEqual(
            [(row["oracle_start"], row["oracle_length"]) for row in result],
            [(2, 2), (4, 4)],
        )
        self.assertEqual(result[0]["hunk_relative_start"], 0)
        self.assertEqual(result[1]["hunk_relative_end"], 6)
        self.assertEqual(result[0]["match_mode"], "exact")
        self.assertEqual(result[1]["match_mode"], "contains")

    def test_unmapped_hunk_is_explicit_and_does_not_persist_private_text(self):
        private = "private oracle text"
        result = INVENTORY.oracle_query_segments(
            1,
            {"oracle_start": 0, "oracle_len": len(private)},
            private,
            [],
        )

        self.assertEqual(result[0]["oracle_event"], None)
        self.assertEqual(result[0]["query_id"], "H000001U001")
        self.assertNotIn(private, json.dumps({key: value for key, value in result[0].items() if key != "private_text"}))

    def test_classification_distinguishes_multi_candidate_from_contiguous_match(self):
        base = [
            {"oracle_event": {"pid": 1}, "inventory": {"match_count": 1}},
            {"oracle_event": {"pid": 2}, "inventory": {"match_count": 1}},
        ]
        self.assertEqual(
            INVENTORY.classify_source_query_segments(base)["label"],
            "continuous_multi_node",
        )
        base[1]["inventory"]["match_count"] = 2
        self.assertEqual(
            INVENTORY.classify_source_query_segments(base)["label"],
            "multiple_candidates",
        )

    def test_pinned_hunks_reconcile_raw_stream_digests_and_offsets(self):
        oracle = "oracle"
        folio = "folio"
        report = {
            "comparison": {
                "oracle": {
                    "raw_unicode_scalar_count": len(oracle),
                    "raw_sha256_audit_only": INVENTORY.DIFF.sha256_text(oracle),
                },
                "folioforge": {
                    "raw_unicode_scalar_count": len(folio),
                    "raw_sha256_audit_only": INVENTORY.DIFF.sha256_text(folio),
                },
                "hunks": [
                    {
                        "kind": "replace",
                        "oracle_start": 0,
                        "oracle_len": len(oracle),
                        "folio_start": 0,
                        "folio_len": len(folio),
                    }
                ],
            }
        }

        result = INVENTORY.validate_pinned_streams(report, oracle, folio)

        self.assertEqual(len(result), 1)
        report["comparison"]["oracle"]["raw_sha256_audit_only"] = "stale"
        with self.assertRaisesRegex(ValueError, "digest"):
            INVENTORY.validate_pinned_streams(report, oracle, folio)

    def test_query_match_rows_remain_content_free(self):
        secret = "PRIVATE-ORACLE-ONLY-BLOCK-82f9"
        entries = [
            {
                "string_id": "S0000001",
                "private_text": "prefix " + secret + " suffix",
                "fragment_index": 3,
                "fragment_type": 145,
                "entity_id": 901,
                "source_path": "$146[7]",
                "parent_field_ids": [146],
                "role_candidate": "string_table_entry_candidate",
                "unicode_scalar_length": 44,
                "sha256_raw_audit_only": "digest",
                "string_table_id": 42,
                "string_table_index": 7,
                "reference_count": 1,
                "references": [],
            }
        ]

        result = INVENTORY.source_string_matches(secret, entries)
        serialized = json.dumps(result)

        self.assertEqual(result["match_count"], 1)
        self.assertNotIn(secret, serialized)
        self.assertNotIn("private_text", serialized)

    def test_query_match_exact_count_survives_display_limit(self):
        entries = [
            {
                "string_id": f"S000000{i}",
                "private_text": None,
                "unicode_scalar_length": 3,
                "sha256_raw_audit_only": "same",
                "references": [],
            }
            for i in range(20)
        ]
        result = INVENTORY.query_source_matches(
            "Q1",
            entries,
            limit=2,
            already_filtered=True,
            exact_length=3,
            exact_sha256="same",
        )

        self.assertEqual(result["match_count"], 20)
        self.assertEqual(result["exact_match_count"], 20)
        self.assertEqual(len(result["matches"]), 2)
        self.assertTrue(result["truncated"])

    def test_private_block_match_preserves_content_role_metadata(self):
        block = "PRIVATE-ORACLE-ONLY-BLOCK-82f9"
        entries = [
            {
                "string_id": "S0000001",
                "private_text": "prefix " + block + " suffix",
                "fragment_index": 3,
                "fragment_type": 145,
                "entity_id": 901,
                "source_path": "$146[7]",
                "parent_field_ids": [146],
                "role_candidate": "string_table_entry_candidate",
                "unicode_scalar_length": 44,
                "sha256_raw_audit_only": "digest",
                "string_table_id": 42,
                "string_table_index": 7,
                "reference_count": 1,
                "references": [
                    {
                        "fragment_index": 5,
                        "fragment_type": 259,
                        "entity_id": 903,
                        "content_fragment_alias": "F002",
                        "source_path": "$146[4].$145",
                        "parent_field_ids": [146, 145],
                        "role_candidate": "content_fragment_string_reference_candidate",
                    }
                ],
            }
        ]

        result = INVENTORY.source_string_matches(block, entries)
        serialized = json.dumps(result)

        self.assertEqual(result["match_count"], 1)
        self.assertEqual(result["matches"][0]["match_kind"], "substring_in_source_string")
        self.assertEqual(result["matches"][0]["content_fragment_reference_count"], 1)
        self.assertEqual(
            result["matches"][0]["content_fragment_references"][0]["source_path"],
            "$146[4].$145",
        )
        self.assertNotIn(block, serialized)
        self.assertNotIn("private_text", serialized)


if __name__ == "__main__":
    unittest.main()
