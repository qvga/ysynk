#!/usr/bin/env python3
"""Build the Linux x86-64 release and assemble its three downloadable assets."""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def archive_permissions(member):
    member.uid = member.gid = 0
    member.uname = member.gname = "root"
    member.mode = 0o755 if member.isdir() or member.mode & 0o111 else 0o644
    return member


def main():
    if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
        raise SystemExit("Build the Linux x86-64 release on Linux x86-64.")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    for manifest in ["package.json", "desktop/tauri.conf.json"]:
        if json.loads((ROOT / manifest).read_text())["version"] != version:
            raise SystemExit(f"{manifest} must match workspace version {version}")
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT
    )
    release = Path(json.loads(metadata)["target_directory"]) / "release"
    subprocess.run(["sh", "packaging/build-deb.sh"], cwd=ROOT, check=True)
    package = release / "bundle/deb" / f"ysynk_{version}_amd64.deb"
    package_version = subprocess.check_output(["dpkg-deb", "-f", package, "Version"], text=True)
    if package_version.strip() != version:
        raise SystemExit(f"Unexpected Debian package version {package_version.strip()}")

    with tempfile.TemporaryDirectory(dir=release) as staging:
        folder = Path(staging) / f"ysynk-{version}-linux-x86_64"
        for source, target in [
            (release / "ysynk-desktop", "ysynk-desktop"),
            (ROOT / "packaging/archive-readme.txt", "README.txt"),
            (ROOT / "LICENSE", "licenses/YSYNK-LICENSE"),
            (ROOT / "docs/RESTIC-LICENSE", "licenses/RESTIC-LICENSE"),
            (ROOT / "target/THIRD_PARTY_NOTICES.txt", "licenses/THIRD_PARTY_NOTICES.txt"),
            (ROOT / "tools/install-desktop-entry.py", "tools/install-desktop-entry.py"),
            (ROOT / "desktop/icons/128x128.png", "desktop/icons/128x128.png"),
            (ROOT / "packaging/ysynk.desktop", "packaging/ysynk.desktop"),
        ]:
            dest = folder / target
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(source, dest)
        archive = Path(staging) / (folder.name + ".tar.gz")
        with tarfile.open(archive, "w:gz") as tar:
            tar.add(folder, arcname=folder.name, filter=archive_permissions)
        checksums = []
        for asset in [package, archive]:
            with asset.open("rb") as stream:
                checksums.append(f"{hashlib.file_digest(stream, 'sha256').hexdigest()}  {asset.name}\n")
        shutil.copyfile(package, Path(staging) / package.name)
        (Path(staging) / "SHA256SUMS").write_text("".join(checksums))
        # Each build gets a separate directory, including rebuilds of a published version.
        assets = Path(tempfile.mkdtemp(prefix=f"assets-{version}-", dir=release))
        for filename in [package.name, archive.name, "SHA256SUMS"]:
            (Path(staging) / filename).replace(assets / filename)
    subprocess.run(["sha256sum", "--check", "SHA256SUMS"], cwd=assets, check=True)
    print(f"Release assets: {assets}")


if __name__ == "__main__":
    main()
