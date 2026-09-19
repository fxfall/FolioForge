import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-all-hunk-audit.py")
SPEC = importlib.util.spec_from_file_location("run_c2_all_hunk_audit", SCRIPT)
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)


def event(trace_id, start, text):
    return {
        "trace_id": trace_id,
        "section_id": "S001",
        "story_id": "ST001",
        "source_fragment": "F001",
        "source_fragment_order": 0,
        "eid": 17,
        "source_path": "$146[0].$145",
        "source_value_kind": "string_reference",
        "source_position": 24,
        "source_position_kind": "field_155_kfx_navigation_target_id",
        "context": "Story",
        "context_basis": "reading_order_section_story_reference",
        "text_len": len(text),
        "start": start,
        "end": start + len(text),
        "text_hash_raw_audit_only": AUDIT.DIFF.sha256_text(text),
        "text": text,
    }


class C2AllHunkAuditTests(unittest.TestCase):
    def test_event_intersections_split_a_multi_event_oracle_span(self):
        events = [
            event("T000001", 0, "abcde"),
            event("T000002", 5, "fghij"),
        ]

        result = AUDIT.event_intersections(2, 6, events)

        self.assertEqual([(row["trace_id"], row["overlap_length"]) for row in result], [
            ("T000001", 3),
            ("T000002", 3),
        ])

    def test_zero_width_side_is_anchored_without_fabricating_text(self):
        events = [event("T000001", 0, "abc")]

        result = AUDIT.side_analysis(3, 0, "", events)
        serialized = json.dumps(result)

        self.assertEqual(result["event_partition"], "single_event")
        self.assertEqual(result["events"][0]["overlap_length"], 0)
        self.assertNotIn("abc", serialized)
        self.assertNotIn('"text"', serialized)

    def test_direction_keeps_both_sides_for_replacement(self):
        oracle_events = [event("O000001", 0, "oracle")]
        folio_events = [event("T000001", 0, "folio")]
        hunk = {
            "kind": "replace",
            "oracle_start": 0,
            "oracle_len": 6,
            "folio_start": 0,
            "folio_len": 5,
            "signature": "replace_length_changed",
            "oracle_classes": AUDIT.DIFF.scalar_categories("oracle"),
            "folio_classes": AUDIT.DIFF.scalar_categories("folio"),
        }

        result = AUDIT.audit_hunk(1, hunk, "oracle", "folio", oracle_events, folio_events)

        self.assertEqual(result["direction"], "oracle_extra")
        self.assertEqual(result["oracle"]["length"], 6)
        self.assertEqual(result["folioforge"]["length"], 5)
        self.assertEqual(result["evidence"]["root_cause_status"], "UnknownRootCause")
        self.assertNotIn("oracle", json.dumps(result["oracle"]))
        self.assertNotIn("folio", json.dumps(result["folioforge"]))

    def test_pinned_stream_validation_reconciles_and_audits_all_hunks(self):
        oracle = "oracle"
        folio = "folio"
        oracle_events = [event("O000001", 0, oracle)]
        folio_events = [event("T000001", 0, folio)]
        report = {
            "comparison": {
                "oracle": {
                    "raw_unicode_scalar_count": len(oracle),
                    "raw_sha256_audit_only": AUDIT.DIFF.sha256_text(oracle),
                    "text_event_count": 1,
                },
                "folioforge": {
                    "raw_unicode_scalar_count": len(folio),
                    "raw_sha256_audit_only": AUDIT.DIFF.sha256_text(folio),
                    "text_event_count": 1,
                },
                "hunks": [{
                    "kind": "replace",
                    "oracle_start": 0,
                    "oracle_len": len(oracle),
                    "folio_start": 0,
                    "folio_len": len(folio),
                    "signature": "replace_length_changed",
                    "oracle_classes": AUDIT.DIFF.scalar_categories(oracle),
                    "folio_classes": AUDIT.DIFF.scalar_categories(folio),
                }],
            }
        }

        result = AUDIT.validate_pinned_streams(report, oracle, folio, oracle_events, folio_events)

        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["direction"], "oracle_extra")


if __name__ == "__main__":
    unittest.main()
