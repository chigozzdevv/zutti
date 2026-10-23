import copy
import unittest

from plan import BENCHMARKS, ROOT, build_plan, read, repeated


class PlanningTests(unittest.TestCase):
    def setUp(self):
        self.inventory = read(BENCHMARKS / "source-inventory/testnet-complete-2026-10-23.json")
        self.distribution = read(ROOT / "testnet-distribution.json")
        self.trees = read(ROOT / "testnet-tree-counts.json")
        self.coinbases = read(ROOT / "testnet-coinbase-counts.json")
        self.reports = [read(ROOT / "results/2026-10-23" / (name + ".json"))
                        for name in ("orchard-original", "orchard-fixed", "ironwood")]
        self.components = read(BENCHMARKS / "source-validation-components-2026-10-23.json")
        self.old = [read(BENCHMARKS / name) for name in (
            "source-workloads-2026-10-19.json", "source-authorization-2026-10-19.json",
            "source-sapling-signatures-2026-10-19.json", "source-trees-2026-10-19.json")]

    def plan(self):
        return build_plan(self.inventory, self.distribution, self.trees, self.coinbases,
                          self.reports, self.components, *self.old)

    def test_scenarios_include_all_components_without_claiming_a_budget(self):
        result = self.plan()
        totals = result["modeled_source_execution_cycles"]
        self.assertGreater(totals["conservative_common_proxy_scenario"], totals["batched_common_proxy_scenario"])
        self.assertGreater(totals["batched_common_proxy_scenario"], result["modern_shared_setup_projected_cycles"])
        self.assertEqual(len(result["components"]), 7)
        self.assertFalse(result["scenarios_are_guaranteed_bounds"])
        self.assertFalse(result["full_bootstrap_budget_established"])
        self.assertIn("Recursive proving and proof compression", result["excluded"])

    def test_coinbase_snapshot_mismatch_rejected(self):
        self.coinbases["target_hash"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "different history"):
            self.plan()

    def test_impossible_coinbase_counts_rejected(self):
        self.coinbases["identified_coinbase_counts"]["sapling_outputs"] = self.inventory["totals"]["sapling_outputs"] + 1
        with self.assertRaisesRegex(ValueError, "exceed total"):
            self.plan()

    def test_duplicate_component_case_rejected(self):
        self.components["cases"].append(copy.deepcopy(self.components["cases"][0]))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            self.plan()

    def test_incompatible_repetition_programs_rejected(self):
        row = next(case for case in self.components["cases"] if case["case"] == "sapling-1-r16")
        row["program_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "incompatible identities"):
            repeated(self.components["cases"], "sapling-1")


if __name__ == "__main__":
    unittest.main()
