import argparse
from fractions import Fraction
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
KINDS = {
    "OrchardOriginal": "orchard-original",
    "OrchardFixed": "orchard-fixed",
    "Ironwood": "ironwood",
}
VERSIONS = {
    "OrchardOriginal": "InsecurePreNu6_2",
    "OrchardFixed": "FixedPostNu6_2",
    "Ironwood": "PostNu6_3",
}


def load(path):
    data = path.read_bytes()
    return json.loads(data), hashlib.sha256(data).hexdigest()


def integer(value):
    if type(value) is not int or value < 0:
        raise ValueError("workload counters must be nonnegative integers")
    return value


def rounded(value):
    return (value.numerator + value.denominator // 2) // value.denominator


def fit_authorization(report):
    if report.get("suite") not in KINDS or report.get("native_authorization_verified") is not True \
            or report.get("journal_matches") is not True or report.get("proof_generated") is not False:
        raise ValueError("authorization fit requires completed native and guest checks")
    cases = report["cases"]
    if len(cases) != 2 or any(case["circuit_version"] != VERSIONS[report["suite"]] for case in cases):
        raise ValueError("authorization fit requires two workloads for the same circuit")
    small, large = cases
    first = [integer(value) for value in small["verification_cycles"]]
    second = [integer(value) for value in large["verification_cycles"]]
    actions = integer(small["actions"]), integer(large["actions"])
    if min(first + second) == 0 or len(first) < 2 or not second or not 0 < actions[0] < actions[1]:
        raise ValueError("authorization fit lacks cold, warm or action-size measurements")
    warm_small = Fraction(sum(first[1:]), len(first) - 1)
    warm_large = Fraction(sum(second), len(second))
    per_action = (warm_large - warm_small) / (actions[1] - actions[0])
    per_bundle = warm_small - actions[0] * per_action
    setup = first[0] - warm_small
    if min(per_action, per_bundle, setup) < 0:
        raise ValueError("samples do not support a nonnegative affine cost model")
    return {"setup": setup, "per_bundle": per_bundle, "per_action": per_action}


def validate_inventory(inventory, distribution, trees):
    binding = inventory["binding"]
    if inventory.get("complete") is not True or inventory.get("groups_truncated") is not False \
            or inventory["totals"]["blocks"] != binding["target_height"] + 1:
        raise ValueError("cycle estimation requires a complete inventory")
    if any(data["run_id"] != inventory["run_id"] or data["network"] != binding["network"]
           for data in (distribution, trees)):
        raise ValueError("cycle estimation cannot mix inventory snapshots")
    if trees["target_hash"] != binding["target_hash"] or trees["target_height"] != binding["target_height"]:
        raise ValueError("tree counts have a different pinned target")
    if distribution["batch_blocks"] != 128 or trees["batch_blocks"] != 128 \
            or distribution["blocks_start_at"] != 1 or trees["blocks_start_at"] != 1:
        raise ValueError("unsupported source batch partition")
    totals = {row["_id"]: row for row in distribution["fixed_128_block_batches"]}
    multi = {row["_id"]: row for row in distribution["multiple_bundle_totals"]}
    if set(totals) != set(KINDS.values()) or set(multi) != set(totals):
        raise ValueError("a modern verification type is missing from the distribution")
    for kind, total in totals.items():
        single = [row for row in distribution["single_bundle_histogram"] if row["_id"]["kind"] == kind]
        bundles = sum(integer(row["bundles"]) for row in single) + integer(multi[kind]["bundles"])
        actions = sum(integer(row["bundles"]) * integer(row["_id"]["actions"]) for row in single) + integer(multi[kind]["actions"])
        if bundles != total["bundles"] or actions != total["actions"]:
            raise ValueError("bundle histograms do not reconcile")
    for field in ("bundles", "actions"):
        if totals["orchard-original"][field] + totals["orchard-fixed"][field] != inventory["totals"]["orchard_" + field] \
                or totals["ironwood"][field] != inventory["totals"]["ironwood_" + field]:
            raise ValueError("distribution does not reconcile with the full inventory")
    return totals


def estimate(inventory, distribution, trees, reports):
    totals = validate_inventory(inventory, distribution, trees)
    if len(reports) != 3 or {report["suite"] for report in reports} != set(KINDS):
        raise ValueError("all three circuit versions must be measured")
    if len({report["program_sha256"] for report in reports}) != 1 \
            or any(report["source_sha256"] != reports[0]["source_sha256"] for report in reports[1:]):
        raise ValueError("authorization measurements have different program or source identities")
    rows = []
    modern = Fraction(0)
    single_block = Fraction(0)
    for report in reports:
        kind = KINDS[report["suite"]]
        rates = fit_authorization(report)
        counts = totals[kind]
        cost = rates["setup"] * counts["batches"] + rates["per_bundle"] * counts["bundles"] + rates["per_action"] * counts["actions"]
        single = sum(row["bundles"] for row in distribution["single_bundle_histogram"] if row["_id"]["kind"] == kind)
        multiple = next(row["blocks"] for row in distribution["multiple_bundle_totals"] if row["_id"] == kind)
        active_blocks = single + multiple
        split_cost = cost + rates["setup"] * (active_blocks - counts["batches"])
        modern += cost
        single_block += split_cost
        rows.append({"kind": kind, "bundles": counts["bundles"], "actions": counts["actions"],
                     "fixed_128_block_key_initializations": counts["batches"],
                     "single_block_key_initializations": active_blocks,
                     "fitted_rates": {key: rounded(value) for key, value in rates.items()},
                     "estimated_cycles": rounded(cost), "single_block_scenario_cycles": rounded(split_cost)})
    if sum(row["single_block_key_initializations"] for row in rows if row["kind"].startswith("orchard-")) != trees["counts"]["orchard_active_blocks"] \
            or next(row["single_block_key_initializations"] for row in rows if row["kind"] == "ironwood") != trees["counts"]["ironwood_active_blocks"]:
        raise ValueError("active-block distributions do not reconcile")
    return {"scope":"Modern authorization component extrapolation, excluding other validation work and proving.",
            "network":inventory["binding"]["network"],"target_height":inventory["binding"]["target_height"],
            "target_hash":inventory["binding"]["target_hash"],"unit":"RISC Zero 3.0.5 user cycles",
            "modern_authorization": rows, "modern_authorization_estimated_cycles": rounded(modern),
            "modern_authorization_single_block_scenario_cycles": rounded(single_block),
            "minimum_128_block_source_batches": (inventory["binding"]["target_height"] + 127) // 128,
            "estimate_is_full_history_execution_count": False,
            "partition_is_proven_feasible": False,
            "proof_generated": False, "paid_jobs": 0}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--measurements", type=Path, nargs=3, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists():
        parser.error("estimate output already exists")
    paths = [ROOT.parent / "source-inventory/testnet-complete-2026-10-23.json",
             ROOT / "testnet-distribution.json", ROOT / "testnet-tree-counts.json"]
    datasets = [load(path) for path in paths]
    measured = [load(path) for path in args.measurements]
    result = estimate(*(data for data, _ in datasets), [data for data, _ in measured])
    result["evidence_sha256"] = {name: checksum for name, (_, checksum) in zip(
        ["inventory", "distribution", "tree_counts"] + [data["suite"] for data, _ in measured], datasets + measured)}
    args.out.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
