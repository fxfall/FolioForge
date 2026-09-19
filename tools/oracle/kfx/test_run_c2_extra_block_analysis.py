import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-extra-block-analysis.py")
SPEC = importlib.util.spec_from_file_location("run_c2_extra_block_analysis", SCRIPT)
ANALYSIS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYSIS)


class C2ExtraBlockAnalysisTests(unittest.TestCase):
    def test_pinned_hunk_offsets_require_exact_raw_stream_reconciliation(self):
        oracle = "abc"
        folio = "abcX"
        report = {
            "comparison": {
                "oracle": {
                    "raw_unicode_scalar_count": len(oracle),
                    "raw_sha256_audit_only": ANALYSIS.DIFF.sha256_text(oracle),
                },
                "folioforge": {
                    "raw_unicode_scalar_count": len(folio),
                    "raw_sha256_audit_only": ANALYSIS.DIFF.sha256_text(folio),
                },
                "hunks": [
                    {
                        "kind": "insert",
                        "oracle_start": 3,
                        "oracle_len": 0,
                        "folio_start": 3,
                        "folio_len": 1,
                    }
                ],
            }
        }

        result = ANALYSIS.validate_pinned_streams(report, oracle, folio)

        self.assertEqual(result[0]["folio_start"], 3)
        report["comparison"]["folioforge"]["raw_sha256_audit_only"] = "wrong-digest"
        with self.assertRaisesRegex(ValueError, "digest"):
            ANALYSIS.validate_pinned_streams(report, oracle, folio)

    def test_occurrences_are_non_overlapping_and_content_free(self):
        block = "PRIVATE-ROOT-CAUSE-MARKER-7c91"
        result = ANALYSIS.repeated_block_summary(
            block,
            {"T000001"},
            block + block + "--" + block,
            [
                {
                    "trace_id": "T000001",
                    "source_fragment": "F001",
                    "section_id": "S001",
                    "story_id": "ST001",
                    "text": block + block,
                },
                {
                    "trace_id": "T000002",
                    "source_fragment": "F002",
                    "section_id": "S001",
                    "story_id": "ST001",
                    "text": block + "--" + block,
                },
            ],
            block,
            1,
        )
        serialized = json.dumps(result)

        self.assertEqual(result["occurrences_in_native_stream_nonoverlapping"], 3)
        self.assertEqual(result["occurrences_in_oracle_stream_nonoverlapping"], 1)
        self.assertEqual(result["other_native_event_occurrence_count"], 2)
        self.assertEqual(result["other_native_events"][0]["trace_id"], "T000002")
        self.assertNotIn(block, serialized)
        self.assertNotIn("text", serialized)

    def test_scalar_categories_are_counts_not_private_text(self):
        categories = ANALYSIS.block_categories("A 7\u00a0")

        self.assertEqual(categories, {"letter": 1, "number": 1, "space": 2})
        self.assertNotIn("A", json.dumps(categories))


if __name__ == "__main__":
    unittest.main()
