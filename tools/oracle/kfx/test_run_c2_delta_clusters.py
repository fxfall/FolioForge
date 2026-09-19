import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-delta-clusters.py")
SPEC = importlib.util.spec_from_file_location("run_c2_delta_clusters", SCRIPT)
CLUSTERS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLUSTERS)


def report(book_id, oracle_count, folio_count, hunks, features=None):
    return {
        "input_id": book_id,
        "source_sha256_audit_only": "digest",
        "comparison": {
            "oracle": {"raw_unicode_scalar_count": oracle_count},
            "folioforge": {
                "raw_unicode_scalar_count": folio_count,
                "source_features": features or {},
            },
            "hunks": hunks,
        },
    }


class C2DeltaClusterTests(unittest.TestCase):
    def test_folioforge_extra_is_a_count_signature_not_a_root_cause(self):
        private = "private unmatched string"
        source = report(
            "KFX-C008",
            100,
            103,
            [
                {
                    "kind": "insert",
                    "signature": "folio_extra",
                    "oracle_len": 0,
                    "folio_len": 3,
                    "oracle_classes": {"counts": {}},
                    "folio_classes": {"counts": {"letter": 2, "number": 1}},
                    "folio_source_provenance": {
                        "events": [{
                            "trace_id": "T000001",
                            "source_value_kind": "direct_text",
                            "source_path": "$146[9].$584",
                        }],
                        "section_ids": ["S001"],
                        "story_ids": ["ST001"],
                    },
                }
            ],
            {
                "has_stories": True,
                "has_footnotes": None,
                "creator": private,
                "undecoded_features": ["footnotes"],
            },
        )

        result = CLUSTERS.summarize_count_mismatch(source)
        serialized = json.dumps(result)

        self.assertEqual(result["direction"], "folioforge_extra")
        self.assertEqual(result["absolute_delta"], 3)
        self.assertEqual(result["root_cause_status"], "UnknownRootCause")
        self.assertEqual(result["folioforge_hunk_scalar_category_counts"], {"letter": 2, "number": 1})
        self.assertEqual(result["hunk_source_value_kind_counts"], {"direct_text": 1})
        self.assertEqual(result["hunk_source_leaf_field_id_counts"], {"$584": 1})
        self.assertIsNone(result["feature_signals"]["has_footnotes"])
        self.assertNotIn(private, serialized)

    def test_missing_case_separates_delete_and_replacement_scalars(self):
        source = report(
            "KFX-C022",
            104,
            103,
            [
                {
                    "kind": "delete",
                    "signature": "folio_missing",
                    "oracle_len": 1,
                    "folio_len": 0,
                    "oracle_classes": {"counts": {"space": 1}},
                    "folio_classes": {"counts": {}},
                    "folio_source_provenance": {"events": [], "section_ids": [], "story_ids": []},
                },
                {
                    "kind": "replace",
                    "signature": "equal_length_oracle_whitespace_replaced",
                    "oracle_len": 3,
                    "folio_len": 3,
                    "oracle_classes": {"counts": {"space": 3}},
                    "folio_classes": {"counts": {"number": 1, "punctuation": 2}},
                    "folio_source_provenance": {"events": [], "section_ids": [], "story_ids": []},
                },
            ],
        )

        result = CLUSTERS.summarize_count_mismatch(source)

        self.assertEqual(result["direction"], "folioforge_missing")
        self.assertEqual(result["delta_folio_minus_oracle"], -1)
        self.assertEqual(result["deleted_oracle_scalars"], 1)
        self.assertEqual(result["replaced_oracle_scalars"], 3)
        self.assertEqual(result["replaced_folio_scalars"], 3)

    def test_equal_count_report_is_not_a_count_mismatch(self):
        self.assertIsNone(CLUSTERS.summarize_count_mismatch(report("KFX-C002", 8, 8, [])))


if __name__ == "__main__":
    unittest.main()
