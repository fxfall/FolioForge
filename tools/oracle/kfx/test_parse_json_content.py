import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("parse-json-content.py")
SPEC = importlib.util.spec_from_file_location("parse_json_content", SCRIPT)
PARSER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PARSER)


class ParseJsonContentTests(unittest.TestCase):
    def test_keeps_sorted_image_occurrences_and_multiplicity_without_text(self):
        secret_text = "copyrighted text must not be emitted"
        raw = {
            "version": 1,
            "data": [
                {"type": 2, "position": 9, "content": "resource-b.jpg"},
                {"type": 1, "position": 1, "content": secret_text},
                {"type": 2, "position": 2, "content": "resource-a.jpg"},
                {"type": 2, "position": 7, "content": "resource-a.jpg"},
                {"type": 7, "position": 8, "content": secret_text},
            ],
        }
        report = PARSER.parse_oracle(raw, "KFX-PL-TEST", "a" * 64, "9.14")

        self.assertEqual(
            report["image_occurrences"],
            [
                {"position": 2, "resource": "img_0001"},
                {"position": 7, "resource": "img_0001"},
                {"position": 9, "resource": "img_0002"},
            ],
        )
        self.assertEqual(
            report["resource_frequencies"],
            [
                {"resource": "img_0001", "count": 2},
                {"resource": "img_0002", "count": 1},
            ],
        )
        self.assertNotIn(secret_text, json.dumps(report))

    def test_aliases_are_independent_of_source_record_order(self):
        first = {
            "data": [
                {"type": 2, "position": 3, "content": "z.jpg"},
                {"type": 2, "position": 1, "content": "a.jpg"},
            ]
        }
        second = {"data": list(reversed(first["data"]))}
        report_a = PARSER.parse_oracle(first, "KFX-PL-TEST", "b" * 64, "9.14")
        report_b = PARSER.parse_oracle(second, "KFX-PL-TEST", "b" * 64, "9.14")
        self.assertEqual(report_a, report_b)


if __name__ == "__main__":
    unittest.main()
