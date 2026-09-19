import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-text-diff.py")
SPEC = importlib.util.spec_from_file_location("run_c2_text_diff", SCRIPT)
DIFF = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIFF)


def event(trace_id, start, text, path="$146[0].$145"):
    return {
        "trace_id": trace_id,
        "section_id": "S001",
        "story_id": "ST001",
        "source_fragment": "F001",
        "source_fragment_order": 0,
        "eid": 17,
        "source_path": path,
        "source_value_kind": "string_reference",
        "source_position": 24,
        "source_position_kind": "field_155_kfx_navigation_target_id",
        "context": "Story",
        "context_basis": "reading_order_section_story_reference",
        "text_len": len(text),
        "start": start,
        "end": start + len(text),
        "text_hash_raw_audit_only": DIFF.sha256_text(text),
        "text": text,
    }


class C2TextDiffTests(unittest.TestCase):
    def test_oracle_events_sort_by_pid_then_keep_source_row_order(self):
        raw = {
            "data": [
                {"type": 1, "position": 8, "content": "😀"},
                {"type": 2, "position": 1, "content": "not body"},
                {"type": 1, "position": 2, "content": "A"},
                {"type": 1, "position": 2, "content": "B"},
            ]
        }
        text, events = DIFF.raw_oracle_events(raw)

        self.assertEqual(text, "AB😀")
        self.assertEqual([item["pid"] for item in events], [2, 2, 8])
        self.assertEqual([item["row_order"] for item in events], [2, 3, 0])
        self.assertEqual(events[-1]["length"], 1)
        self.assertNotIn("not body", json.dumps([{k: v for k, v in e.items() if k != "content"} for e in events]))

    def test_hunks_map_native_offsets_to_numeric_provenance_without_text(self):
        private = "prefix PRIVATE-MARKER suffix"
        oracle = "prefix PRIVATE-SYMBOL suffix"
        folio_events = [event("T000001", 0, private)]
        hunks, summary = DIFF.make_hunks(oracle, private, folio_events)
        serialized = json.dumps({"hunks": hunks, "summary": summary})

        self.assertEqual(summary["first_divergent_unicode_index"], 15)
        self.assertGreaterEqual(summary["hunk_count"], 1)
        self.assertEqual(hunks[0]["folio_source_provenance"]["events"][0]["source_path"], "$146[0].$145")
        self.assertNotIn(private, serialized)
        self.assertNotIn(oracle, serialized)
        self.assertNotIn("text", hunks[0])

    def test_hunk_signature_marks_equal_length_whitespace_replacement(self):
        self.assertEqual(
            DIFF.hunk_signature("replace", "   ", "abc"),
            "equal_length_oracle_whitespace_replaced",
        )
        self.assertEqual(
            DIFF.hunk_signature("replace", "\u00a0", " "),
            "equal_length_non_ascii_space_candidate",
        )

    def test_secondary_normalizations_never_replace_raw_result(self):
        diagnostics = DIFF.normalization_diagnostics("e\u0301", "é")

        self.assertFalse(diagnostics["raw_equal"])
        self.assertTrue(diagnostics["nfc_only_equal"])
        self.assertEqual(DIFF.book_signatures("e\u0301", "é", {
            "equal_length_replacements_only": True,
            "hunk_count": 1,
        }, diagnostics, []), ["unicode-normalization-difference"])

    def test_raw_stream_reconciles_event_scalars_and_digest(self):
        text = "e\u0301😀"
        report = {
            "events": [event("T000001", 0, text)],
            "stream": {
                "raw_unicode_scalar_count": len(text),
                "raw_sha256_audit_only": DIFF.sha256_text(text),
            },
        }
        stream, events = DIFF.native_event_stream(report)

        self.assertEqual(stream, text)
        self.assertEqual(len(stream), 3)
        self.assertEqual(events[0]["end"], 3)

    def test_insertion_and_deletion_keep_direction_explicit(self):
        folio_events = [event("T000001", 0, "abcXdef")]
        insert_hunks, insert_summary = DIFF.make_hunks("abcdef", "abcXdef", folio_events)
        delete_hunks, delete_summary = DIFF.make_hunks("abcXdef", "abcdef", folio_events)

        self.assertEqual(insert_hunks[0]["kind"], "insert")
        self.assertEqual(insert_summary["changed_scalar_delta_folio_minus_oracle"], 1)
        self.assertEqual(delete_hunks[0]["kind"], "delete")
        self.assertEqual(delete_summary["changed_scalar_delta_folio_minus_oracle"], -1)

    def test_large_repetitive_stream_is_localized_without_character_quadratic_diff(self):
        oracle = "chapter " * 100_000
        changed_at = 400_000
        folio = oracle[:changed_at] + "X" + oracle[changed_at + 1 :]

        hunks, summary = DIFF.make_hunks(
            oracle, folio, [event("T000001", 0, folio)]
        )

        self.assertEqual(summary["first_divergent_unicode_index"], changed_at)
        self.assertEqual(summary["hunk_count"], 1)
        self.assertEqual(hunks[0]["oracle_start"], changed_at)
        self.assertEqual(hunks[0]["folio_start"], changed_at)
        self.assertEqual(hunks[0]["oracle_len"], 1)
        self.assertEqual(hunks[0]["folio_len"], 1)


if __name__ == "__main__":
    unittest.main()
