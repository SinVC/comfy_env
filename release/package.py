"""Create release archives, checksums, and version-pinned installers."""

import argparse
import hashlib
from pathlib import Path
import re
import tarfile
import tomllib
import zipfile

REPO = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-pc-windows-msvc": ("gov.exe", ".zip"),
    "x86_64-unknown-linux-musl": ("gov", ".tar.gz"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--binary", type=Path)
    args = parser.parse_args()
    version = tomllib.loads((REPO / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if not re.fullmatch(r"v\d+\.\d+\.\d+", args.tag) or args.tag != f"v{version}":
        parser.error(f"expected stable release tag v{version}, got {args.tag}")
    if bool(args.binary) != bool(args.target):
        parser.error("--target and --binary must be provided together")

    args.output.mkdir(parents=True, exist_ok=True)
    if not args.target:
        for name in ("install.sh", "install.ps1"):
            script = (REPO / "release" / name).read_text(encoding="utf-8")
            (args.output / name).write_text(
                script.replace("@RELEASE_TAG@", args.tag), encoding="utf-8", newline="\n"
            )
        return

    binary_name, extension = TARGETS[args.target]
    archive = args.output / f"gov-{args.target}{extension}"
    files = [
        (args.binary, binary_name),
        (REPO / "config.toml.template", "config.toml.template"),
        (REPO / "pyproject.toml.template", "pyproject.toml.template"),
    ]
    if extension == ".zip":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
            for source, name in files:
                bundle.write(source, name)
    else:
        with tarfile.open(archive, "w:gz") as bundle:
            for source, name in files:
                bundle.add(source, arcname=name)
    with archive.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(digest + "\n", encoding="ascii")


if __name__ == "__main__":
    main()
