import argparse
import hashlib
import json
from pathlib import Path
import tarfile


VENDOR = Path(__file__).resolve().parents[2] / "crates/prover/methods/source/vendor"
ARCHIVES = {
    "halo2_proofs": ("0.4.0", "f5af2a9457f494f862cd07b81c2e98a19f50b06a2057b7569450d7c26b167d1c"),
    "maybe-rayon": ("0.1.1", "8ea1f30cedd69f0a2954655f7188c6a834246d2bcf1e315e2ac40c4b24dc9519"),
}


def expected_patch(crate, name, original):
    if crate == "halo2_proofs" and name in ("src/multicore.rs", "src/plonk/verifier/batch.rs"):
        return original.replace(
            b'feature = "multicore"', b'all(feature = "multicore", not(target_os = "zkvm"))',
        )
    if crate == "maybe-rayon" and name == "src/lib.rs":
        before = b'not(feature = "threads"),'
        after = b'target_os = "zkvm", not(feature = "threads"),'
        if original.count(before) != 1:
            raise ValueError("sequential facade configuration changed upstream")
        return original.replace(before, after)
    return original


def check(cache):
    checked = []
    for crate, (version, checksum) in ARCHIVES.items():
        archive = cache / f"{crate}-{version}.crate"
        if hashlib.sha256(archive.read_bytes()).hexdigest() != checksum:
            raise ValueError(f"{crate}: upstream archive digest mismatch")
        directory = VENDOR / crate
        provenance = json.loads((directory / "provenance.json").read_text())
        if provenance["upstream_archive_sha256"] != checksum or provenance["version"] != version:
            raise ValueError(f"{crate}: provenance mismatch")
        prefix = f"{crate}-{version}/"
        names = set()
        changes = []
        with tarfile.open(archive, "r:gz") as source:
            for member in source:
                if not member.isfile():
                    continue
                if not member.name.startswith(prefix) or member.size > 2_000_000:
                    raise ValueError("unexpected registry archive member")
                name = member.name[len(prefix):]
                if Path(name).is_absolute() or ".." in Path(name).parts:
                    raise ValueError("invalid registry archive path")
                names.add(name)
                original = source.extractfile(member).read()
                actual = (directory / name).read_bytes()
                if actual != expected_patch(crate, name, original):
                    raise ValueError(f"{crate}/{name}: unexpected vendored change")
                if actual != original:
                    changes.append({"file": name,
                                    "upstream_sha256": hashlib.sha256(original).hexdigest(),
                                    "patched_sha256": hashlib.sha256(actual).hexdigest()})
        actual_names = {str(path.relative_to(directory)) for path in directory.rglob("*") if path.is_file()}
        if actual_names != names | {"provenance.json"}:
            raise ValueError(f"{crate}: vendored file set mismatch")
        if sorted(changes, key=lambda row: row["file"]) != sorted(provenance["changes"], key=lambda row: row["file"]):
            raise ValueError(f"{crate}: changed file digests mismatch")
        checked.append({"crate": crate, "version": version, "files": len(names), "changes": len(changes)})
    return checked


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cache", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(check(args.cache), indent=2))


if __name__ == "__main__":
    main()
