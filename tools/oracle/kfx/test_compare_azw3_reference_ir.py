import importlib.util
import json
import unittest


SCRIPT = __import__("pathlib").Path(__file__).with_name("compare-azw3-reference-ir.py")
SPEC = importlib.util.spec_from_file_location("compare_azw3_reference_ir", SCRIPT)
COMPARISON = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPARISON)


class Azw3ReferenceIrTests(unittest.TestCase):
    def test_summary_contains_hashes_but_not_visible_text(self):
        secret = "private book text"
        payload = {
            "metadata": {"title": "sample", "authors": ["author"]},
            "documents": [
                {
                    "id": 0,
                    "href": "part.xhtml",
                    "media_type": "application/xhtml+xml",
                    "nodes": [
                        {
                            "kind": {"Heading": {"level": 1}},
                            "role": "Heading",
                            "children": [
                                {
                                    "kind": {"Text": {"value": secret}},
                                    "role": "Text",
                                    "children": [],
                                }
                            ],
                        }
                    ],
                }
            ],
            "resources": [{"kind": "Jpeg", "media_type": "image/jpeg", "path": "cover.jpg", "size": 4}],
            "styles": [{"id": 0, "properties": {"font-family": "Serif", "font-size": "1em"}}],
            "font_faces": [],
        }

        result = COMPARISON.summarize_semantic(payload)
        serialized = json.dumps(result, ensure_ascii=False)

        self.assertNotIn(secret, serialized)
        self.assertEqual(result["whole_text_unicode_scalar_count"], len(secret))
        self.assertEqual(
            result["whole_text_whitespace_stripped_unicode_scalar_count"],
            len(secret.replace(" ", "")),
        )
        self.assertEqual(result["heading_count"], 1)
        self.assertEqual(result["resources"]["count"], 1)
        self.assertEqual(result["styles"]["feature_values"]["font-family"], {"Serif": 1})

    def test_compare_reports_text_delta_and_structure_flags(self):
        left = {
            "available": True,
            "document_count": 2,
            "document_href_sequence_sha256": "same-docs",
            "whole_text_sha256": "left",
            "whole_text_normalized_sha256": "left-normalized",
            "whole_text_whitespace_stripped_sha256": "left-no-space",
            "whole_text_unicode_scalar_count": 10,
            "whole_text_whitespace_stripped_unicode_scalar_count": 9,
            "heading_sequence_sha256": "headings",
            "navigation_edges_sha256": "links",
            "navigation_toc": {"toc_count": 1},
            "resources": {"count": 1},
            "styles": {"count": 1},
            "font_face_count": 0,
        }
        right = {
            **left,
            "whole_text_sha256": "right",
            "whole_text_normalized_sha256": "right-normalized",
            "whole_text_unicode_scalar_count": 13,
            "whole_text_whitespace_stripped_unicode_scalar_count": 12,
        }

        result = COMPARISON.compare_summaries(left, right)

        self.assertFalse(result["whole_text_sha256_equal"])
        self.assertFalse(result["whole_text_normalized_sha256_equal"])
        self.assertEqual(result["whole_text_unicode_scalar_delta"], 3)
        self.assertEqual(result["whole_text_whitespace_stripped_unicode_scalar_delta"], 3)
        self.assertTrue(result["document_href_sequence_equal"])


if __name__ == "__main__":
    unittest.main()
