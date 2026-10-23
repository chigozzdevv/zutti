import argparse
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
from pathlib import Path

from estimate import estimate, rounded


ROOT = Path(__file__).resolve().parent
BENCHMARKS = ROOT.parent


def read(path):
    return json.loads(path.read_text())


def repeated(cases, name, unit=1):
    first = next(case for case in cases if case["case"] == name + "-r1")
    many = next(case for case in cases if case["case"] == name + "-r16")
    if first["program_sha256"] != many["program_sha256"] or first["block_sha256"] != many["block_sha256"] \
            or first["journal_matches"] is not True or many["journal_matches"] is not True:
        raise ValueError("repetition measurements have incompatible identities")
    warm = Fraction(many["cycles"] - first["cycles"], 15)
    shared = first["cycles"] - warm
    if warm <= 0 or shared < 0:
        raise ValueError("invalid repetition measurements")
    return warm / unit, shared


def build_plan(inventory, distribution, trees, coinbases, reports, components, old_source, old_groth, old_signatures, old_trees):
    modern = estimate(inventory, distribution, trees, reports)
    if coinbases["run_id"] != inventory["run_id"] or coinbases["target_hash"] != inventory["binding"]["target_hash"]:
        raise ValueError("coinbase counts identify a different history")
    counts = inventory["totals"]
    cases = components["cases"]
    if len({case["case"] for case in cases}) != len(cases):
        raise ValueError("component measurements contain duplicate case names")
    source = {row["name"]: row for row in old_source["cases"]}
    small, large = source["index-zero-1000-1016"], source["index-zero-1000-1128"]
    if small["program_sha256"] != large["program_sha256"]:
        raise ValueError("common-work samples use different programs")
    per_block = Fraction(large["execution"]["cycles"] - small["execution"]["cycles"], 112)
    shared = small["execution"]["cycles"] - 16 * per_block
    batches = modern["minimum_128_block_source_batches"]
    blocks = inventory["binding"]["target_height"]
    common_low = per_block * blocks + shared * batches
    common_high = old_signatures["source_coinbase_control"]["execution"]["cycles"] * blocks
    extra_tx = source["launch-60497-60498"]["execution"]["cycles"] - source["launch-60000-60001"]["execution"]["cycles"]
    launch = source["launch-1000-1016"]["execution"]["cycles"]
    phgr_run = old_source["pairing_preparation_comparison"]["candidate_profiled_execution"]["cycles"]
    phgr = Fraction(phgr_run - launch - 3 * extra_tx, 4)
    groth = {row["name"]: row for row in old_groth["cases"]}
    sprout_low = phgr * counts["sprout_phgr_proofs"] + groth["sprout-groth16-windowed"]["cycles"] * counts["sprout_groth16_proofs"]
    sprout_high = phgr * (counts["sprout_phgr_proofs"] + counts["sprout_groth16_proofs"])
    output_proof = Fraction(groth["windowed-64"]["cycles"] - groth["windowed-32"]["cycles"], 32)
    sapling_warm, sapling_shared = repeated(cases, "sapling-1")
    bundle_extra = sapling_warm - output_proof
    signatures = next(row for row in old_signatures["cases"] if row["accelerated"] and row["count"] == 4)
    signature_proxy = Fraction(signatures["cycles"], signatures["count"])
    spend_proxy = groth["sapling-spend-windowed"]["cycles"] + signature_proxy
    sapling_low = output_proof * counts["sapling_outputs"] + bundle_extra * counts["sapling_bundles"] \
        + spend_proxy * counts["sapling_spends"] + sapling_shared * trees["counts"]["sapling_occupied_batches"]
    sapling_high = sapling_low + bundle_extra * (counts["sapling_outputs"] + counts["sapling_spends"])
    tree_cost = Fraction(0)
    for pool, larger in (("orchard", 4), ("ironwood", 7)):
        warm, _ = repeated(cases, f"trees-{pool}-1")
        one = next(row for row in cases if row["case"] == f"trees-{pool}-1-r16")
        many = next(row for row in cases if row["case"] == f"trees-{pool}-{larger}-r16")
        if one["program_sha256"] != many["program_sha256"]:
            raise ValueError("tree-size samples use different programs")
        per_action = Fraction(many["cycles"] - one["cycles"], 16 * (larger - 1))
        per_changed_block = warm - per_action
        tree_cost += per_changed_block * trees["counts"][pool + "_active_blocks"] + per_action * counts[pool + "_actions"]
    sapling_tree, sapling_tree_shared = repeated(cases, "trees-sapling-1")
    tree_cost += sapling_tree * trees["counts"]["sapling_active_blocks"] \
        + Fraction(old_trees["comparison"]["accelerated_paired_additional_cycles"], old_trees["comparison"]["additional_hashes"]) * (counts["sapling_outputs"] - trees["counts"]["sapling_active_blocks"]) \
        + sapling_tree_shared * trees["counts"]["sapling_occupied_batches"]
    recovery_low = Fraction(0)
    recovery_high = Fraction(0)
    for pool, name, unit in (("sapling", "context-sapling-1", 1),
                             ("orchard", "context-orchard-original-2", 2),
                             ("ironwood", "context-ironwood-1", 1)):
        rate, initial = repeated(cases, name, unit)
        field = pool + ("_outputs" if pool == "sapling" else "_actions")
        identified = coinbases["identified_coinbase_counts"][field]
        if not 0 <= identified <= counts[field]:
            raise ValueError("identified coinbases exceed total actions")
        occupied = trees["counts"][pool + "_occupied_batches"]
        recovery_low += rate * identified + initial * occupied
        recovery_high += rate * counts[field] + initial * occupied
    state_low = extra_tx * counts["non_coinbase_transactions"]
    state_high = extra_tx * (counts["transparent_inputs"] + counts["transparent_outputs"] - blocks)
    rows = [
        ("modern_authorization", modern["modern_authorization_estimated_cycles"], modern["modern_authorization_estimated_cycles"]),
        ("common_block_work_proxy", common_low, common_high),
        ("legacy_sprout_authorization_proxy", sprout_low, sprout_high),
        ("sapling_authorization_proxy", sapling_low, sapling_high),
        ("shielded_tree_update_projection", tree_cost, tree_cost),
        ("shielded_coinbase_context_proxy", recovery_low, recovery_high),
        ("additional_transaction_ledger_script_proxy", state_low, state_high),
    ]
    totals = [sum(Fraction(row[index]) for row in rows) for index in (1, 2)]
    setup = sum(Fraction(report["cases"][0]["verification_cycles"][0] - report["cases"][0]["verification_cycles"][1]) * row["fixed_128_block_key_initializations"]
                for report, row in zip(reports, modern["modern_authorization"]))
    return {
        "schema": 1, "scope": "Exploratory source-execution planning scenarios for the pinned public Testnet history.",
        "network": "testnet", "target_height": blocks, "target_hash": inventory["binding"]["target_hash"],
        "unit": "RISC Zero 3.0.5 user cycles", "batch_blocks_assumed": 128,
        "components": [{"component": name, "batched_common_proxy_cycles": rounded(Fraction(low)),
                        "conservative_common_proxy_cycles": rounded(Fraction(high))} for name, low, high in rows],
        "modeled_source_execution_cycles": {"batched_common_proxy_scenario": rounded(totals[0]),
                                            "conservative_common_proxy_scenario": rounded(totals[1])},
        "modern_shared_setup_projected_cycles": rounded(setup), "modern_authorization_model": modern,
        "assumptions": [
            "Fixed 128-block groups share verifier setup. This partition has not been shown to fit all witness and execution bounds.",
            "Modern verification uses affine fits from the measured 1/2-action or 2/6-action bundles. Larger bundles are extrapolated.",
            "Common work uses an early 16/128-block source fit; the conservative scenario charges the measured 91,899,147-cycle single-block control for every block.",
            "Legacy source rates and component samples come from different recorded program versions. Their use here is a transfer assumption, not a matched whole-source execution.",
            "Sprout and Sapling proxies include conservative allowances for formats without an isolated warm transaction measurement.",
            "Recovery scenarios charge identified coinbases versus all shielded outputs/actions. Shared recovery setup is charged in every pool-occupied batch.",
            "Additional state/script work uses a historical transparent-transaction delta per extra transaction or per extra transparent state operation. Executed multisig counts and representative later-state costs remain unmeasured.",
            "Tree fits use synthetic frontiers. Historical frontier reconstruction and overlap with common-work proxies remain uncertain.",
        ],
        "excluded": ["Additional setup caused by batch splits", "Larger-bundle fit error", "Initial genesis receipt generation",
                     "Reorganisations and retries", "Segment padding and paging overhead", "Recursive proving and proof compression",
                     "Ethereum wrapping and verification", "Per-swap private proofs", "Provider fees and execution/proving throughput"],
        "scenarios_are_guaranteed_bounds": False, "full_history_vm_execution_performed": False,
        "full_bootstrap_budget_established": False, "source_state_admitted": False,
        "proof_generated": False, "paid_jobs": 0,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists():
        parser.error("planning output already exists")
    paths = [BENCHMARKS / "source-inventory/testnet-complete-2026-10-23.json", ROOT / "testnet-distribution.json",
             ROOT / "testnet-tree-counts.json", ROOT / "testnet-coinbase-counts.json"]
    measured = [ROOT / "results/2026-10-23" / (name + ".json") for name in ("orchard-original", "orchard-fixed", "ironwood")]
    rest = [BENCHMARKS / "source-validation-components-2026-10-23.json", BENCHMARKS / "source-workloads-2026-10-19.json",
            BENCHMARKS / "source-authorization-2026-10-19.json", BENCHMARKS / "source-sapling-signatures-2026-10-19.json",
            BENCHMARKS / "source-trees-2026-10-19.json"]
    result = build_plan(*(read(path) for path in paths), [read(path) for path in measured], *(read(path) for path in rest))
    result["evidence_sha256"] = {str(path.relative_to(BENCHMARKS)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in paths + measured + rest}
    result["computed_utc"] = datetime.now(timezone.utc).isoformat()
    result["model_sha256"] = {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                              for name in ("plan.py", "estimate.py")}
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"modeled_source_execution_cycles": result["modeled_source_execution_cycles"],
                      "modern_shared_setup_projected_cycles": result["modern_shared_setup_projected_cycles"],
                      "components": result["components"]}, indent=2))


if __name__ == "__main__":
    main()
