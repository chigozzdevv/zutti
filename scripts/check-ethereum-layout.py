#!/usr/bin/env python3
import json
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parent.parent
result = subprocess.run(
    ["forge", "inspect", "--root", str(root / "contracts"), "PrivatePool", "storage-layout", "--json"],
    check=True, capture_output=True, text=True,
)
layout = json.loads(result.stdout)
expected = {"spent": "69", "commitments": "70", "orders": "71"}
for name, slot in expected.items():
    entry = next(item for item in layout["storage"] if item["label"] == name)
    kind = layout["types"][entry["type"]]
    if entry["slot"] != slot or entry["offset"] != 0 or kind["label"] != "mapping(bytes32 => bool)":
        raise SystemExit(f"Ethereum proof verifier storage layout changed: {name}")
print("Ethereum reservation storage layout verified")
