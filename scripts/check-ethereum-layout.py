#!/usr/bin/env python3
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parent.parent
profiles = {
    "PrivatePool": {"spent": (69, 0, "mapping(bytes32 => bool)"),
                    "commitments": (70, 0, "mapping(bytes32 => bool)"),
                    "orders": (71, 0, "mapping(bytes32 => bool)")},
    "SpvPool": {"roots": (68, 0, "mapping(bytes32 => bool)"),
                "spent": (69, 0, "mapping(bytes32 => bool)"),
                "commitments": (70, 0, "mapping(bytes32 => bool)"),
                "orders": (71, 0, "mapping(bytes32 => bool)")},
    "SpvRegistry": {"stableHeight": (2, 0, "uint32"), "headTimestamp": (2, 4, "uint32"),
                    "proposedAt": (4, 0, "uint64"), "halted": (4, 8, "bool"),
                    "_accepted": (19, 0, "mapping(bytes32 => bool)")},
}
for contract, expected in profiles.items():
    result = subprocess.run(
        ["forge", "inspect", "--root", str(root / "contracts"), contract, "storage-layout", "--json", "--force"],
        check=True, capture_output=True, text=True,
    )
    layout = json.loads(result.stdout)
    for name, (slot, offset, label) in expected.items():
        entry = next(item for item in layout["storage"] if item["label"] == name)
        kind = layout["types"][entry["type"]]
        if int(entry["slot"]) != slot or entry["offset"] != offset or kind["label"] != label:
            raise SystemExit(f"Ethereum proof verifier storage layout changed: {contract}.{name}")
print("Ethereum reservation and SPV inventory storage layouts verified")
