import copy
import json
from pathlib import Path
import unittest

from estimate import VERSIONS, estimate, fit_authorization


ROOT = Path(__file__).resolve().parent


def measurement(suite, small, large):
    return {
        "suite": suite, "native_authorization_verified": True, "journal_matches": True,
        "proof_generated": False, "program_sha256": "a" * 64, "source_sha256": {"source": "b" * 64},
        "cases": [
            {"circuit_version": VERSIONS[suite], "actions": small,
             "verification_cycles": [1000 + 60 + 40 * small, 60 + 40 * small]},
            {"circuit_version": VERSIONS[suite], "actions": large,
             "verification_cycles": [60 + 40 * large, 60 + 40 * large]},
        ],
    }


class EstimateTests(unittest.TestCase):
    def setUp(self):
        self.inventory = json.loads((ROOT.parent / "source-inventory/testnet-complete-2026-10-23.json").read_text())
        self.distribution = json.loads((ROOT / "testnet-distribution.json").read_text())
        self.trees = json.loads((ROOT / "testnet-tree-counts.json").read_text())
        self.reports = [measurement("OrchardOriginal", 2, 6), measurement("OrchardFixed", 1, 2), measurement("Ironwood", 1, 2)]

    def run_estimate(self):
        return estimate(self.inventory, self.distribution, self.trees, self.reports)

    def test_recovers_shared_setup_and_action_work(self):
        fit = fit_authorization(self.reports[0])
        self.assertEqual(fit, {"setup": 1000, "per_bundle": 60, "per_action": 40})

    def test_setup_charged_per_occupied_batch(self):
        result = self.run_estimate()
        expected = sum(1000 * row["batches"] + 60 * row["bundles"] + 40 * row["actions"]
                       for row in self.distribution["fixed_128_block_batches"])
        self.assertEqual(result["modern_authorization_estimated_cycles"], expected)
        self.assertFalse(result["partition_is_proven_feasible"])
        self.assertFalse(result["estimate_is_full_history_execution_count"])

    def test_missing_circuit_never_counts_as_zero(self):
        self.reports.pop()
        with self.assertRaisesRegex(ValueError, "all three"):
            self.run_estimate()

    def test_split_batches_repeat_setup_work(self):
        result = self.run_estimate()
        self.assertGreater(result["modern_authorization_single_block_scenario_cycles"],
                           result["modern_authorization_estimated_cycles"])

    def test_mislabeled_circuit_rejected(self):
        self.reports[0]["cases"][0]["circuit_version"] = VERSIONS["Ironwood"]
        with self.assertRaisesRegex(ValueError, "same circuit"):
            self.run_estimate()

    def test_truncated_census_rejected(self):
        self.inventory["groups_truncated"] = True
        with self.assertRaisesRegex(ValueError, "complete inventory"):
            self.run_estimate()

    def test_different_history_target_rejected(self):
        self.trees["target_hash"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "different pinned target"):
            self.run_estimate()

    def test_counts_must_reconcile(self):
        self.distribution["multiple_bundle_totals"][0]["bundles"] += 1
        with self.assertRaisesRegex(ValueError, "do not reconcile"):
            self.run_estimate()

    def test_mixed_programs_rejected(self):
        self.reports[1]["program_sha256"] = "c" * 64
        with self.assertRaisesRegex(ValueError, "different program"):
            self.run_estimate()

    def test_mixed_sources_rejected(self):
        self.reports[1]["source_sha256"]["source"] = "c" * 64
        with self.assertRaisesRegex(ValueError, "different program or source"):
            self.run_estimate()

    def test_failed_or_capped_execution_is_not_a_complete_measurement(self):
        report = copy.deepcopy(self.reports[0])
        report["journal_matches"] = False
        report["cycles_lower_bound"] = 1 << 36
        with self.assertRaisesRegex(ValueError, "completed native and guest checks"):
            fit_authorization(report)

    def test_unsupported_linear_fit_rejected(self):
        report = copy.deepcopy(self.reports[0])
        report["cases"][1]["verification_cycles"] = [1, 1]
        with self.assertRaisesRegex(ValueError, "nonnegative"):
            fit_authorization(report)


if __name__ == "__main__":
    unittest.main()
