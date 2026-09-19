import importlib.util
import json
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-style-correlation.py")
SPEC = importlib.util.spec_from_file_location("run_c2_style_correlation", SCRIPT)
CORRELATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CORRELATION)


class C2StyleCorrelationTests(unittest.TestCase):
    def test_exact_native_symbol_617_range_is_classified_without_text(self):
        source_event = {
            "trace_id": "T000001",
            "source_fragment": "F006",
            "source_path": "$146[84].$145",
            "start": 100,
            "end": 110,
            "adjacent_style_events": [
                {
                    "list_index": 0,
                    "text_offset": 4,
                    "text_length": 3,
                    "style_symbol_id": 617,
                }
            ],
        }
        hunk = {
            "kind": "replace",
            "folio_start": 104,
            "folio_len": 3,
            "folio_source_provenance": {
                "events": [
                    {
                        "trace_id": "T000001",
                        "source_fragment": "F006",
                        "source_path": "$146[84].$145",
                        "start": 100,
                    }
                ]
            },
        }

        result = CORRELATION.classify_hunk(hunk, {"T000001": source_event})

        self.assertEqual(result["classification"], "exact_symbol_617_range")
        self.assertEqual(result["exact_style_ranges"][0]["local_scalar_offset"], 4)
        self.assertNotIn("text", json.dumps(result))

    def test_symbol_617_partial_overlap_is_not_reported_as_exact(self):
        source_event = {
            "trace_id": "T000001",
            "source_fragment": "F006",
            "source_path": "$146[84].$145",
            "start": 100,
            "end": 110,
            "adjacent_style_events": [
                {"list_index": 0, "text_offset": 3, "text_length": 4, "style_symbol_id": 617}
            ],
        }
        hunk = {
            "kind": "replace",
            "folio_start": 104,
            "folio_len": 2,
            "folio_source_provenance": {
                "events": [
                    {
                        "trace_id": "T000001",
                        "source_fragment": "F006",
                        "source_path": "$146[84].$145",
                        "start": 100,
                    }
                ]
            },
        }

        result = CORRELATION.classify_hunk(hunk, {"T000001": source_event})

        self.assertEqual(result["classification"], "symbol_617_overlap_not_exact")
        self.assertEqual(result["exact_style_ranges"], [])

    def test_mismatched_source_provenance_stays_unresolved(self):
        hunk = {
            "kind": "replace",
            "folio_start": 104,
            "folio_len": 1,
            "folio_source_provenance": {
                "events": [
                    {
                        "trace_id": "T000001",
                        "source_fragment": "F999",
                        "source_path": "$146[84].$145",
                        "start": 100,
                    }
                ]
            },
        }

        result = CORRELATION.classify_hunk(hunk, {"T000001": {"source_fragment": "F006"}})

        self.assertEqual(result["classification"], "source_event_unresolved")
        self.assertEqual(result["unresolved_trace_ids"], ["T000001"])


if __name__ == "__main__":
    unittest.main()
