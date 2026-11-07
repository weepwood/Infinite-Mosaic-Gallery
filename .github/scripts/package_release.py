#!/usr/bin/env python3
"""Package the native executable without bundling incompatible system libraries.

Uses only Python stdlib. Called after cargo build --release on each OS runner.
"""
from __future__ import annotations

import hashlib
import platform
import tarfile
import tomllib
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[2]
with (root / "Cargo.toml").open("rb") as stream:
    version = tomllib.load(stream)["package"]["version"]

system = {"Linux": "linux", "Darwin": "macos", "Windows": "windows"}[platform.system()]
arch = platform.machine().lower()
arch = {"amd64": "x86_64", "x64": "x86_64", "aarch64": "arm64"}.get(arch, arch)
application = "infinite-mosaic-gallery"
executable = root / "target" / "release" / (application + (".exe" if system == "windows" else ""))
if not executable.is_file():
    raise SystemExit(f"Missing release executable: {executable}")

package_name = f"{application}-v{version}-{system}-{arch}"
output_dir = root / "dist"
output_dir.mkdir(exist_ok=True)
members = [
    (executable, executable.name),
    (root / "README.md", "README.md"),
    (root / "docs" / "RELEASE-v0.4.1.md", "RELEASE-NOTES.md"),
]

if system == "windows":
    archive = output_dir / f"{package_name}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        for file, name in members:
            output.write(file, arcname=f"{package_name}/{name}")
else:
    archive = output_dir / f"{package_name}.tar.gz"
    with tarfile.open(archive, "w:gz") as output:
        for file, name in members:
            output.add(file, arcname=f"{package_name}/{name}")

digest = hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()
(output_dir / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
print(f"Packaged {archive.name} ({archive.stat().st_size} bytes, sha256 {digest})")
