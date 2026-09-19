import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("run-c2-minimum-source-evidence.py")
SPEC = importlib.util.spec_from_file_location("run_c2_minimum_source_evidence", SCRIPT)
EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVIDENCE)


class C2MinimumSourceEvidenceTests(unittest.TestCase):
    def test_sample_paths_cover_prioritized_cases_without_body_text(self):
        self.assertEqual(
            set(EVIDENCE.SAMPLE_PATHS),
            {"KFX-C022", "KFX-C059", "KFX-C075", "KFX-C082"},
        )
        self.assertEqual(sum(len(paths) for paths in EVIDENCE.SAMPLE_PATHS.values()), 7)

    def test_content_free_guard_rejects_private_text_keys(self):
        with self.assertRaisesRegex(ValueError, "private fields"):
            EVIDENCE.assert_content_free({"text": "private"})

    def test_content_free_guard_accepts_hash_and_scalar_summary(self):
        EVIDENCE.assert_content_free(
            {
                "resolved_text": {
                    "sha256_raw_audit_only": "digest",
                    "unicode_scalar_count": 37,
                },
                "path_context": [{"parent_path": "$146[0]"}],
            }
        )


if __name__ == "__main__":
    unittest.main()
