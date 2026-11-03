#!/usr/bin/env python3
"""Collect release notices offline; --fetch-missing prepares a pinned upstream cache."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parent.parent
PREFIXES = ("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE")
JAVASCRIPT_PACKAGES = (
    "node_modules/svelte", "node_modules/@tauri-apps/api",
    "node_modules/clsx", "node_modules/esm-env", "vendor/deepmerge",
    "node_modules/vite", "node_modules/rolldown",
)
# This crate omits its repository field; its packaged VCS commit identifies this upstream.
REPOSITORIES = {"libappindicator-sys": "https://github.com/tauri-apps/libappindicator-rs"}
# These exact versions have explicit grants but omit the referenced standard text.
STANDARD_TEXTS = {
    ("io_tee", "0.1.1"): ("README.md", "Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0.txt"),
    ("selectors", "0.38.0"): ("selectors/lib.rs", "MPL-2.0", "https://www.mozilla.org/media/MPL/2.0/index.txt"),
}


def public_url(value):
    value = value.removeprefix("git+").rstrip("/").removesuffix(".git")
    if value.startswith("git://github.com/"):
        value = value.replace("git://", "https://", 1)
    parts = urlsplit(value)
    if parts.scheme != "https" or not parts.hostname or parts.username or parts.password:
        raise ValueError("upstream source must be a public HTTPS URL")
    if parts.query or parts.fragment:
        raise ValueError("upstream source must not contain query parameters or fragments")
    return value


def collect_files(root, license_file=None):
    """Read only root notice files and an explicitly declared, contained license file."""
    root = root.resolve()
    files = {p for p in root.iterdir() if p.name.upper().startswith(PREFIXES) and p.is_file()}
    if license_file:
        files.add(root / license_file)
    result = {}
    for path in sorted(files):
        if not path.resolve().is_relative_to(root):
            raise ValueError("license path escapes package root")
        text = path.read_bytes().decode("utf-8")
        if not text.strip():
            raise ValueError("empty license/notice file: " + path.name)
        result[path.relative_to(root).as_posix()] = text
    return result


def upstream(package, root):
    source = package.get("source") or ""
    repository = package.get("repository") or REPOSITORIES.get(package["name"])
    commit = None
    if source.startswith("git+"):
        repository = source[4:].split("?", 1)[0].split("#", 1)[0]
        commit = source.rsplit("#", 1)[-1]
    elif (root / ".cargo_vcs_info.json").is_file():
        commit = json.loads((root / ".cargo_vcs_info.json").read_text())["git"]["sha1"]
    if commit and not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("invalid upstream commit")
    repository = public_url(repository) if repository else None
    return repository, commit


def fetch_json(url):
    request = Request(url, headers={"User-Agent": "ysynk-release-notices/0.5.1"})
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def supplement(package, repository, commit, cache, fetch):
    """Use root/crate notice blobs, never a recursive scan of upstream source."""
    if not repository or not commit or not repository.startswith("https://github.com/"):
        raise ValueError("missing license files and no pinned GitHub upstream")
    root = Path(package["manifest_path"]).parent
    vcs_file = root / ".cargo_vcs_info.json"
    subdir = json.loads(vcs_file.read_text()).get("path_in_vcs", "") if vcs_file.exists() else ""
    if subdir and (Path(subdir).is_absolute() or ".." in Path(subdir).parts):
        raise ValueError("invalid upstream crate directory")
    key = hashlib.sha256(f"{repository}@{commit}/{subdir}/{package['name']}".encode()).hexdigest()
    path = cache / (key + ".json")
    standard = STANDARD_TEXTS.get((package["name"], package["version"]))
    if not path.exists():
        if not fetch:
            raise ValueError("missing license files; prepare cache with --fetch-missing")
        repo = repository.removeprefix("https://github.com/")
        def list_notices(directory):
            url = f"https://api.github.com/repos/{repo}/contents/{directory}?ref={commit}"
            for entry in fetch_json(url):
                if not entry["name"].upper().startswith(PREFIXES):
                    continue
                if entry["type"] == "file":
                    yield entry["path"]
                elif entry["type"] == "dir":
                    # REUSE-style LICENSES directories contain license texts only.
                    for child in fetch_json(f"https://api.github.com/repos/{repo}/contents/{entry['path']}?ref={commit}"):
                        if child["type"] == "file":
                            yield child["path"]

        records = []
        names = set(list_notices(""))
        if subdir:
            names.update(list_notices(subdir))
        urls = [(name, f"https://raw.githubusercontent.com/{repo}/{commit}/{name}") for name in sorted(names)]
        if not urls and standard:
            grant, identifier, url = standard
            urls = [("GRANT-" + identifier, f"https://raw.githubusercontent.com/{repo}/{commit}/{grant}"),
                    ("LICENSE-" + identifier, url)]
        for name, url in urls:
            with urlopen(url, timeout=30) as response:
                text = response.read().decode("utf-8")
            if standard and name == "GRANT-MPL-2.0":
                text = text.split("*/", 1)[0] + "*/\n"
            records.append({"name": name, "url": url, "text": text,
                            "sha256": hashlib.sha256(text.encode()).hexdigest()})
        cache.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"repository": repository, "commit": commit,
                                    "files": records}, indent=2) + "\n")
    data = json.loads(path.read_text())
    if data["repository"] != repository or data["commit"] != commit:
        raise ValueError("upstream notice cache provenance mismatch")
    files = {}
    for item in data["files"]:
        name, text = item["name"], item["text"]
        expected = f"https://raw.githubusercontent.com/{repository.split('github.com/', 1)[1]}/{commit}/{name}"
        if standard:
            grant, identifier, url = standard
            if name == "GRANT-" + identifier:
                expected = f"https://raw.githubusercontent.com/{repository.split('github.com/', 1)[1]}/{commit}/{grant}"
            elif name == "LICENSE-" + identifier:
                expected = url
        if (Path(name).is_absolute() or ".." in Path(name).parts
                or not any(part.upper().startswith(PREFIXES + ("GRANT-",)) for part in Path(name).parts)
                or item["url"] != expected or not text.strip()
                or hashlib.sha256(text.encode()).hexdigest() != item["sha256"]):
            raise ValueError("invalid upstream notice cache file")
        files[f"upstream/{name} ({expected})"] = text
    if not files:
        raise ValueError("pinned upstream has no root license files")
    return files


def section(name, version, license_expression, sources, files, has_declared_license=False):
    if not license_expression:
        raise ValueError("missing declared license expression")
    if not has_declared_license and not any(any(part.upper().startswith(("LICENSE", "LICENCE", "COPYING"))
                   for part in Path(name.split(" (", 1)[0]).parts)
               for name in files):
        raise ValueError("no license text (copyright/NOTICE alone is insufficient)")
    result = [f"\n{'=' * 72}\n{name} {version}\nDeclared license: {license_expression}\n"]
    if license_expression == "Apache-2.0 OR GPL-2.0-only":
        result.append("Selected license: Apache-2.0 (not GPL-2.0-only)\n")
    if (name, version) in STANDARD_TEXTS:
        result.append("Upstream omits license text: explicit grant and canonical standard text included.\n")
        result.append("Selected license: " + STANDARD_TEXTS[(name, version)][1] + "\n")
    result.extend(f"Source: {source}\n" for source in sources)
    for filename, text in sorted(files.items()):
        result.append(f"\n--- {filename} ---\n{text}")
        if not text.endswith("\n"):
            result.append("\n")
    return "".join(result)


def generate(metadata, cache, fetch):
    selected = {node["id"] for node in metadata["resolve"]["nodes"]}
    workspace = set(metadata["workspace_members"])
    sections, errors = [], []
    packages = [p for p in metadata["packages"] if p["id"] in selected - workspace]
    for package in sorted(packages, key=lambda p: (p["name"], p["version"])):
        label = f"{package['name']} {package['version']}"
        try:
            root = Path(package["manifest_path"]).parent
            repository, commit = upstream(package, root)
            files = collect_files(root, package.get("license_file"))
            if not files and (package.get("source") or "").startswith("git+"):
                git_root = Path(subprocess.check_output(
                    ["git", "-C", str(root), "rev-parse", "--show-toplevel"], text=True).strip())
                files = {f"repository-root/{name}": text
                         for name, text in collect_files(git_root).items()}
            if not files:
                files = supplement(package, repository, commit, cache, fetch)
            sources = []
            if not (package.get("source") or "").startswith("git+"):
                sources.append(f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download")
            if repository:
                sources.append(repository + (f"/tree/{commit}" if commit else ""))
            if not package.get("source"):
                sources.append("Local backport; see docs/DEPENDENCIES.md and vendor/GLIB_BACKPORT.md")
            sections.append(section(package["name"], package["version"], package["license"], sources, files,
                                    has_declared_license=bool(package.get("license_file"))))
        except (OSError, ValueError, KeyError) as error:
            # Do not print cache paths, metadata IDs, or exception URLs with local credentials.
            detail = str(error) if isinstance(error, ValueError) else type(error).__name__
            errors.append(f"{label}: {detail}")
    for directory in JAVASCRIPT_PACKAGES:
        try:
            root = ROOT / directory
            package = json.loads((root / "package.json").read_text())
            repository = package["repository"]
            repository = repository["url"] if isinstance(repository, dict) else repository
            if re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
                repository = "https://github.com/" + repository
            repository = public_url(repository)
            sources = [f"https://registry.npmjs.org/{package['name']}/-/{package['name'].split('/')[-1]}-{package['version']}.tgz",
                       repository]
            if package.get("gitHead"):
                if not re.fullmatch(r"[0-9a-f]{40}", package["gitHead"]):
                    raise ValueError("invalid upstream JavaScript commit")
                sources.append(repository + "/tree/" + package["gitHead"])
            sections.append(section(package["name"], package["version"], package["license"],
                                    sources, collect_files(root)))
        except (OSError, ValueError, KeyError) as error:
            detail = str(error) if isinstance(error, ValueError) else type(error).__name__
            errors.append(f"{directory}: {detail}")
    if errors:
        raise ValueError("Required third-party notices unavailable:\n" + "\n".join(errors))
    header = ("ysynk 0.5.1 - third-party notices\n"
              "Scope: x86_64-unknown-linux-gnu Cargo resolve graph, excluding workspace crates;\n"
              "overinclusive: includes build/dev dependencies, not a linked-binary inventory.\n"
               "Also includes Svelte, @tauri-apps/api, clsx, esm-env, vendored deepmerge, Vite and Rolldown.\n"
              "This superset accompanies the desktop package; restic's external license is supplied separately.\n"
              "Declared alternatives are preserved; Apache-2.0 is selected over GPL-2.0-only.\n"
              "Upstream license, copyright and NOTICE texts follow without rewriting.\n"
               f"Inventory: {len(packages)} Rust packages; {len(JAVASCRIPT_PACKAGES)} JavaScript packages.\n")
    return header + "".join(sections)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, help="existing Linux cargo metadata JSON")
    parser.add_argument("--output", type=Path, default=ROOT / "target/THIRD_PARTY_NOTICES.txt")
    parser.add_argument("--cache", type=Path, default=ROOT / "target/release-notices/upstream")
    parser.add_argument("--fetch-missing", action="store_true", help="allow pinned upstream license downloads")
    args = parser.parse_args()
    try:
        if args.metadata:
            metadata = json.loads(args.metadata.read_text())
        else:
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--offline", "--filter-platform",
                 "x86_64-unknown-linux-gnu", "--format-version", "1"], cwd=ROOT))
        text = generate(metadata, args.cache, args.fetch_missing)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
        print(f"Generated third-party notices ({len(text.encode())} bytes)")
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error) if isinstance(error, ValueError) else type(error).__name__, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
