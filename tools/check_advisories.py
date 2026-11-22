#!/usr/bin/env python3
"""Query OSV's public RustSec/npm advisory data for exact locked dependencies.

Only public package names/versions are sent. This is not a source-code audit.
Network/database incompleteness is a failed check, never an empty clean report.
"""
import argparse
import json
from pathlib import Path
import sys
import tomllib
import urllib.request


def query(batch):
    data = json.dumps({"queries": batch}).encode()
    request = urllib.request.Request("https://api.osv.dev/v1/querybatch", data=data,
                                     headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)["results"]


def npm_packages(root):
    npm = json.loads((root / "package-lock.json").read_text())
    inventory = npm.get("packages")
    if (npm.get("lockfileVersion") not in (2, 3)
            or not isinstance(inventory, dict) or not isinstance(inventory.get(""), dict)):
        raise ValueError("Unsupported or incomplete npm lockfile package inventory")
    local_targets = set()
    for path, package in inventory.items():
        if "node_modules" not in Path(path).parts:
            continue
        if not isinstance(package, dict):
            raise ValueError(f"Invalid npm lockfile entry: {path}")
        name = package.get("name", path.rsplit("node_modules/", 1)[-1])
        version = package.get("version")
        if package.get("link"):
            target = package.get("resolved")
            if not isinstance(target, str) or not target or Path(target).is_absolute():
                raise ValueError(f"Invalid npm lockfile link target: {path}")
            locked_target = inventory.get(target)
            if not isinstance(locked_target, dict) or locked_target.get("link"):
                raise ValueError(f"Missing npm lockfile link target: {target}")
            manifest_path = (root / target / "package.json").resolve()
            if not manifest_path.is_relative_to(root) or not manifest_path.is_file():
                raise ValueError(f"Missing local npm package manifest: {target}")
            manifest = json.loads(manifest_path.read_text())
            name, version = manifest.get("name"), manifest.get("version")
            if locked_target.get("version") != version or locked_target.get("name", name) != name:
                raise ValueError(f"Local package differs from npm lockfile: {target}")
            local_targets.add(target)
        if not all(isinstance(field, str) and field.strip() for field in (name, version)):
            raise ValueError(f"Missing package name or version in npm lockfile: {path}")
        yield name, version, "npm"
    for path in inventory:
        if path and "node_modules" not in Path(path).parts and path not in local_targets:
            raise ValueError(f"Unreferenced local npm lockfile entry: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    cargo = tomllib.loads((root / "Cargo.lock").read_text())
    packages = {(p["name"], p["version"], "crates.io")
                for p in cargo["package"] if not p['name'].startswith('ysynk-')}
    packages.update(npm_packages(root))
    packages = sorted(packages)
    findings = []
    for offset in range(0, len(packages), 100):
        batch = packages[offset:offset + 100]
        results = query([{"package": {"name": name, "ecosystem": ecosystem}, "version": version}
                         for name, version, ecosystem in batch])
        if len(results) != len(batch) or any(result.get("next_page_token") for result in results):
            raise RuntimeError("Incomplete advisory response; refusing a clean result")
        for (name, version, ecosystem), result in zip(batch, results):
            if result.get("vulns"):
                findings.append({"package": name, "version": version, "ecosystem": ecosystem,
                                 "advisories": [v["id"] for v in result["vulns"]]})
    report = {"database": "OSV public RustSec/npm data", "packages_checked": len(packages),
              "findings": findings,
              "local_patch_notes": ["GLib and deepmerge keep upstream versions: raw findings remain visible; see vendor/*BACKPORT.md for exact source remediation and tests.",
                                    "Pinned Git dependencies are queried by their declared version too; this does not audit untagged commits."],
              "limitations": "Known-version advisory check only; not a security audit or source-patch verifier"}
    text = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.write_text(text)
    print(text, end="")
    return 1 if findings else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(f"Advisory check unavailable: {error}", file=sys.stderr)
        sys.exit(2)
