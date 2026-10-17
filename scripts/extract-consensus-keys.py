import argparse
import hashlib
import json
from pathlib import Path


SAPLING = {
    "spend": "8270785a1a0d0bc77196f000ee6d221c9c9894f55307bd9357c3f0105d31ca63991ab91324160d8f53e2bbd3c2633a6eb8bdf5205d822e7f3f73edac51b2b70c",
    "output": "657e3d38dbb5cb5e7dd2970e8b03d69b4787dd907285b5a7f0790dcc8072f60bf593b32cc2d1c030e00ff5ae64bf84c5c3beb84ddc841d48264b4a171744d028",
}
PHGR = "4bd498dae0aacfd8e98dc306338d017d9c08dd0918ead18172bd0aec2fc5df82"
GROTH16 = "4f8894d5456596e912d8a55c138c155f338131ea4ec747847b5dd6dba5efabdc"
MODULUS = 21888242871839275222246405745257275088696311157297823662689037894645226208583


def extract_phgr(data):
    if hashlib.sha256(data).hexdigest() != PHGR:
        raise ValueError("Sprout ceremony key hash mismatch")
    position = 0
    result = bytearray(b"ZUTTIPHG\x01\x00")
    inverse = pow(2**256, -1, MODULUS)

    def point(fields):
        nonlocal position
        if data[position] != 48:
            raise ValueError("invalid libsnark point flag")
        position += 1
        for _ in range(fields):
            value = int.from_bytes(data[position : position + 32], "little")
            position += 32
            if value >= MODULUS:
                raise ValueError("noncanonical libsnark field")
            result.extend((value * inverse % MODULUS).to_bytes(32, "big"))

    for fields in [4, 2, 4, 4, 2, 4, 4, 2]:
        point(fields)
    indices = b"9\n9\n0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n"
    if data[position : position + len(indices)] != indices:
        raise ValueError("unexpected libsnark input query")
    position += len(indices)
    for _ in range(9):
        point(2)
    if position != len(data):
        raise ValueError("trailing libsnark key bytes")
    return bytes(result)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sapling-spend", required=True, type=Path)
    parser.add_argument("--sapling-output", required=True, type=Path)
    parser.add_argument("--sprout-phgr", required=True, type=Path)
    parser.add_argument("--sprout-groth16", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    outputs = {}
    for name, expected in SAPLING.items():
        data = getattr(args, "sapling_" + name).read_bytes()
        if hashlib.blake2b(data).hexdigest() != expected:
            raise ValueError("Sapling parameter hash mismatch: " + name)
        count = int.from_bytes(data[864:868], "big")
        if count != {"spend": 8, "output": 6}[name]:
            raise ValueError("unexpected Sapling input count")
        outputs["sapling-" + name + ".vk"] = data[: 868 + count * 96]
    outputs["sprout-phgr.vk"] = extract_phgr(args.sprout_phgr.read_bytes())
    data = args.sprout_groth16.read_bytes()
    if hashlib.sha256(data).hexdigest() != GROTH16:
        raise ValueError("Sprout Groth16 key hash mismatch")
    outputs["sprout-groth16.vk"] = data
    args.out.mkdir(parents=True, exist_ok=True)
    for name, data in outputs.items():
        (args.out / name).write_bytes(data)
    print(json.dumps({name: hashlib.sha256(data).hexdigest() for name, data in outputs.items()}, indent=2))


if __name__ == "__main__":
    main()
