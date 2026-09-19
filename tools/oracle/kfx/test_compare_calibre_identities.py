import importlib.util
import unittest
from collections import Counter
from pathlib import Path


SCRIPT = Path(__file__).with_name("compare-calibre-identities.py")
SPEC = importlib.util.spec_from_file_location("compare_calibre_identities", SCRIPT)
COMPARISON = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPARISON)


class CapacityMatchingTests(unittest.TestCase):
    def test_ordered_stage_comparison_uses_stage_order_and_exact_identity_only(self):
        occurrences = [
            {"resource": "img_0002", "identity_status": "exact_image_identity"},
            {"resource": "img_0001", "identity_status": "exact_image_identity"},
            {"resource": "img_0003", "identity_status": "unresolved_image_identity"},
        ]
        result = COMPARISON.ordered_stage_comparison(
            occurrences,
            ["fid-b", "fid-a", "fid-c"],
            {"img_0001": "fid-a", "img_0002": "fid-b", "img_0003": "fid-c"},
        )

        self.assertEqual(result["occurrence_count"], 3)
        self.assertEqual(result["ordered_identity_matches"], 2)
        self.assertFalse(result["resource_frequencies_equal_oracle"])

    def test_ordered_stage_comparison_does_not_zip_different_lengths_silently(self):
        result = COMPARISON.ordered_stage_comparison(
            [{"resource": "img_0001", "identity_status": "exact_image_identity"}],
            ["fid-a", "fid-b"],
            {"img_0001": "fid-a"},
        )

        self.assertEqual(result["ordered_identity_matches"], 1)
        self.assertEqual(result["ordered_comparison_count"], 1)
        self.assertFalse(result["resource_frequencies_equal_oracle"])

    def test_capacity_constraints_force_a_unique_candidate(self):
        rows = [{"A"}, {"A", "B"}, {"B"}]
        matched, feasible_counts = COMPARISON.capacity_matching_analysis(
            rows, Counter({"A": 2, "B": 1})
        )

        self.assertEqual(matched, 3)
        self.assertEqual(feasible_counts, [1, 1, 1])

    def test_symmetric_candidates_remain_ambiguous(self):
        rows = [{"A", "B"}, {"A", "B"}]
        matched, feasible_counts = COMPARISON.capacity_matching_analysis(
            rows, Counter({"A": 1, "B": 1})
        )

        self.assertEqual(matched, 2)
        self.assertEqual(feasible_counts, [2, 2])

    def test_incomplete_matching_does_not_claim_forced_associations(self):
        rows = [{"A"}, {"A"}]
        matched, feasible_counts = COMPARISON.capacity_matching_analysis(
            rows, Counter({"A": 1})
        )

        self.assertEqual(matched, 1)
        self.assertIsNone(feasible_counts)

    def test_candidates_absent_from_oracle_are_not_matchable(self):
        rows = [{"not-in-oracle"}, {"A"}]
        matched, feasible_counts = COMPARISON.capacity_matching_analysis(
            rows, Counter({"A": 1})
        )

        self.assertEqual(matched, 1)
        self.assertIsNone(feasible_counts)


if __name__ == "__main__":
    unittest.main()
