#!/usr/bin/env python3
"""Check exact reviewed backport bytes and dependency selections, not security.

Expected hashes/provenance are recorded in vendor/*BACKPORT.md. Changes require
new source review; passing this check does not replace tests or upstream support.
"""
import argparse
import hashlib
import json
from pathlib import Path
import tomllib

EXPECTED = {
    'vendor/glib/src/variant_iter.rs': 'a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc',
    'vendor/deepmerge/index.js': '92d5af8bf2c1a5f75929941600ab839f1473296d4d2bd7d7a2a57389e6528324',
    'vendor/deepmerge/dist/cjs.js': '88121122451e38da55a2c841a30b5abb9d6402a32216d92cbca26eafd520f839',
    'vendor/deepmerge/dist/umd.js': '24deeddeecdfa685d2e89da821e8e98ebfe3ceadf0d4da01bb2ed4fe2f22d5af',
}


def verify_files(root, expected):
    for relative, wanted in expected.items():
        path = root / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'Reviewed source is missing or is a symlink: {relative}')
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != wanted:
            raise ValueError(f'Reviewed source changed; review before updating its expected hash: {relative}')


def check(root):
    verify_files(root, EXPECTED)
    cargo = tomllib.loads((root / 'Cargo.toml').read_text())
    if cargo['patch']['crates-io']['glib'].get('path') != 'vendor/glib':
        raise ValueError('Cargo does not select the reviewed local GLib source')
    glib = tomllib.loads((root / 'vendor/glib/Cargo.toml').read_text())
    merge = json.loads((root / 'vendor/deepmerge/package.json').read_text())
    if glib['package']['version'] != '0.18.5' or merge['version'] != '4.3.1':
        raise ValueError('Backports must retain their true upstream versions')
    return {'scope': 'Exact reviewed source-byte and dependency-selection check, not an audit',
            'verified_source_files': list(EXPECTED),
            'raw_version_findings_suppressed': False,
            'required_separate_checks': ['Backport regressions', 'Actual npm plugin resolution',
                                         'Native keychain integration', 'Raw advisory query']}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    text = json.dumps(check(Path(__file__).resolve().parent.parent), indent=2) + '\n'
    if args.output:
        args.output.write_text(text)
    print(text)
