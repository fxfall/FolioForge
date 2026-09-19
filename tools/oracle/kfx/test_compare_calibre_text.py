import importlib.util
import json
import tempfile
import unittest
import zipfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("compare-calibre-text.py")
SPEC = importlib.util.spec_from_file_location("compare_calibre_text", SCRIPT)
COMPARISON = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPARISON)


class CalibreTextComparisonTests(unittest.TestCase):
    def test_text_is_sorted_then_normalized_without_emitting_text(self):
        secret = "private visible text"
        raw = {
            "data": [
                {"type": 1, "position": 8, "content": "😀"},
                {"type": 2, "position": 7, "content": "private-resource-name.jpg"},
                {"type": 1, "position": 2, "content": "e\u0301\r\n"},
                {"type": 1, "position": 3, "content": "中"},
                {"type": 7, "position": 9, "content": secret},
                {"type": 1, "position": 4, "content": secret},
            ],
        }

        result = COMPARISON.summarize_calibre_text(raw)
        serialized = json.dumps(result)

        self.assertEqual(result["text_fragment_count"], 4)
        self.assertEqual(result["unicode_scalar_count"], 24)
        self.assertNotIn(secret, serialized)
        self.assertNotIn("private-resource-name.jpg", serialized)

    def test_empty_visible_text_has_stable_empty_digest(self):
        result = COMPARISON.summarize_calibre_text({"data": []})

        self.assertEqual(result["text_fragment_count"], 0)
        self.assertEqual(result["unicode_scalar_count"], 0)
        self.assertEqual(
            result["normalized_sha256_audit_only"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        )

    def test_fragment_alignment_reports_first_boundary_without_text(self):
        secret = "abXXef"
        native_units = [
            {
                "content_fragment_order": 0,
                "unicode_scalar_count": 2,
                **COMPARISON.summarize_text("ab"),
            },
            {
                "content_fragment_order": 1,
                "unicode_scalar_count": 2,
                **COMPARISON.summarize_text("cd"),
            },
            {
                "content_fragment_order": 2,
                "unicode_scalar_count": 2,
                **COMPARISON.summarize_text("ef"),
            },
        ]

        result = COMPARISON.calibre_to_native_fragment_alignment(
            secret, native_units, native_scalar_count=6
        )

        self.assertEqual(result["exact_unit_count"], 2)
        self.assertEqual(result["mismatched_unit_count"], 1)
        self.assertEqual(result["first_mismatched_content_fragment_order"], 1)
        self.assertNotIn(secret, json.dumps(result))

    def test_paired_stage_units_distinguish_exact_and_mismatched_hashes(self):
        left = [
            {"document_order": 0, **COMPARISON.summarize_text("one")},
            {"document_order": 1, **COMPARISON.summarize_text("two")},
        ]
        right = [
            {"document_order": 0, **COMPARISON.summarize_text("one")},
            {"document_order": 1, **COMPARISON.summarize_text("xxx")},
        ]

        result = COMPARISON.paired_unit_summary(left, right, "document_order")

        self.assertEqual(result["paired_unit_count"], 2)
        self.assertEqual(result["exact_hash_match_count"], 1)
        self.assertEqual(result["mismatched_hash_count"], 1)

    def test_epub_text_follows_spine_order_and_emits_only_summary(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            epub_path = Path(temporary_directory) / "audit.epub"
            with zipfile.ZipFile(epub_path, "w") as archive:
                archive.writestr(
                    "META-INF/container.xml",
                    '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
                    '<rootfiles><rootfile full-path="OPS/package.opf"/></rootfiles>'
                    "</container>",
                )
                archive.writestr(
                    "OPS/package.opf",
                    '<package xmlns="http://www.idpf.org/2007/opf">'
                    '<manifest><item id="a" href="a.xhtml"/>'
                    '<item id="b" href="b.xhtml"/></manifest>'
                    '<spine><itemref idref="b"/><itemref idref="a"/></spine>'
                    "</package>",
                )
                archive.writestr(
                    "OPS/a.xhtml",
                    '<html xmlns="http://www.w3.org/1999/xhtml"><body>secret-A</body></html>',
                )
                archive.writestr(
                    "OPS/b.xhtml",
                    '<html xmlns="http://www.w3.org/1999/xhtml"><body>e&#769;😀</body></html>',
                )

            result, parsed = COMPARISON.epub_visible_text(epub_path)
            raw_text, raw_document_count, raw_node_count = (
                COMPARISON.epub_visible_text_stream(epub_path)
            )
            serialized = json.dumps(result)

        self.assertTrue(parsed)
        self.assertEqual(result["document_count"], 2)
        self.assertEqual(result["unicode_scalar_count"], 10)
        self.assertEqual(raw_text, "e\u0301😀secret-A")
        self.assertEqual(len(raw_text), 11)
        self.assertEqual(raw_document_count, 2)
        self.assertEqual(raw_node_count, 2)
        self.assertNotIn("secret-A", serialized)


if __name__ == "__main__":
    unittest.main()
