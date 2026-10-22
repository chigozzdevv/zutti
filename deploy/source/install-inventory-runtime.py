import argparse
import hashlib
import json
from pathlib import Path
import stat
import subprocess
import sys
import urllib.request
from urllib.parse import urlsplit
import venv
import zipfile


def fetch(url, limit):
    if not url.startswith(("https://pypi.org/", "https://files.pythonhosted.org/")):
        raise ValueError("unexpected dependency host")
    with urllib.request.urlopen(url, timeout=30) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError("dependency download exceeds its size bound")
    return data


def install(directory, requirements):
    if sys.platform != "linux" or sys.version_info[:2] != (3, 12) or __import__("platform").machine() != "x86_64":
        raise ValueError("this pinned runtime installer requires Linux x86_64 with Python 3.12")
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    environment = directory / "venv"
    venv.EnvBuilder(with_pip=False).create(environment)
    executable = environment / "bin/python"
    site = Path(subprocess.check_output([str(executable), "-c", "import sysconfig; print(sysconfig.get_path('purelib'))"], text=True).strip())
    lock = json.loads(requirements.with_suffix(".lock.json").read_text())
    expected = [tuple(line.strip().split("==")) for line in requirements.read_text().splitlines()]
    if [(row["name"], row["version"]) for row in lock] != expected:
        raise ValueError("dependency lock differs from the pinned requirements")
    downloaded = 0
    for wheel in lock:
        name = Path(urlsplit(wheel["url"]).path).name
        if not name.endswith(".whl"):
            raise ValueError("dependency lock does not identify a wheel")
        archive = directory / name
        if archive.is_file() and hashlib.sha256(archive.read_bytes()).hexdigest() == wheel["sha256"]:
            data = archive.read_bytes()
        else:
            data = fetch(wheel["url"], 8_000_000)
            downloaded += len(data)
        digest = hashlib.sha256(data).hexdigest()
        if digest != wheel["sha256"] or len(data) != wheel["bytes"]:
            raise ValueError("dependency checksum mismatch")
        archive.write_bytes(data)
        with zipfile.ZipFile(archive) as contents:
            for member in contents.infolist():
                path = Path(member.filename)
                if path.is_absolute() or ".." in path.parts or stat.S_ISLNK(member.external_attr >> 16):
                    raise ValueError("unsafe dependency archive member")
            contents.extractall(site)
    subprocess.run([str(executable), "-c", "import pymongo,dns; print(pymongo.version, dns.__version__)"], check=True)
    (directory / "dependencies.json").write_text(json.dumps(lock, indent=2) + "\n")
    print(json.dumps({"runtime": str(environment), "downloads_bytes": downloaded}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--requirements", required=True, type=Path)
    args = parser.parse_args()
    install(args.directory, args.requirements)
