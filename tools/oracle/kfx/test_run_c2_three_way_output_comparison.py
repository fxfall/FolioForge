import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-three-way-output-comparison.py")
SPEC = importlib.util.spec_from_file_location("run_c2_three_way_output_comparison", SCRIPT)
COMPARISON = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPARISON)


class C2ThreeWayOutputComparisonTests(unittest.TestCase):
    def test_pairwise_comparison_keeps_raw_and_secondary_hashes_separate(self):
        left = {
            "raw_unicode_scalar_count": 3,
            "raw_sha256_audit_only": "raw-left",
            "normalized_unicode_scalar_count": 2,
            "normalized_sha256_audit_only": "normalized-shared",
        }
        right = {
            "raw_unicode_scalar_count": 4,
            "raw_sha256_audit_only": "raw-right",
            "normalized_unicode_scalar_count": 2,
            "normalized_sha256_audit_only": "normalized-shared",
        }

        result = COMPARISON.pairwise(left, right)

        self.assertTrue(result["comparable"])
        self.assertFalse(result["raw_scalar_count_equal"])
        self.assertFalse(result["raw_hash_equal"])
        self.assertTrue(result["normalized_scalar_count_equal"])
        self.assertTrue(result["normalized_hash_equal"])

    def test_baseline_comparison_is_content_free(self):
        secret = "private body text"
        current = {
            "normalized_unicode_scalar_count": 5,
            "normalized_sha256_audit_only": "digest",
        }
        baseline = {
            "unicode_scalar_count": 5,
            "normalized_sha256_audit_only": "digest",
            "private_text": secret,
        }

        result = COMPARISON.baseline_comparison(current, baseline)

        self.assertTrue(result["normalized_hash_equal"])
        self.assertEqual(result["normalized_scalar_delta_current_minus_pinned"], 0)
        self.assertNotIn(secret, json.dumps(result))

    def test_invalid_epub_is_reported_without_retaining_converter_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "broken.epub"
            output.write_bytes(b"not an epub")

            result = COMPARISON.run_converter(
                ["/usr/bin/true"],
                output,
            )

        self.assertEqual(result, {"status": "invalid_epub"})


if __name__ == "__main__":
    unittest.main()
