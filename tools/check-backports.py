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
    'vendor/glib/src/collections/list.rs': 'e23dd1ca864011a291104dbcb35baca101d98b049e83eee9d0a03381ce1dc1cc',
    'vendor/glib/src/collections/slist.rs': '7d3cd457f8b2d1c56ba710525de4fbddec0628886b3ba34c2813a6e352716075',
    'vendor/glib/src/enums.rs': '284beec75075a21ffccebd6947ba2d92c5ffcba4a2c933585e5b3081211ae718',
    'vendor/glib/src/main_context.rs': '3ef005e199eb56e74af8b7fea9a575cec4fca8af36721bd7ade236979f34fda6',
    'vendor/glib/src/object.rs': '8ca4267d2fd4355f49e5f820fc6b291ad0ab81757bf533a352308b7faf5cde93',
    'vendor/glib/src/param_spec.rs': '60f2c331f3578729fe9acb4b4653f38cba1a29b01d7e0cca0bba9aaae05bd755',
    'vendor/glib/src/subclass/types.rs': 'db093d5ca3025408ef842410e66f73075eb763029053b1dfe288b20032e66ef9',
    'vendor/glib/src/translate.rs': 'a3d9dcbdc3399f58007e491db40ce173459b0d3ec837c731c313ffbe6d3a1cf7',
    'vendor/glib/src/value.rs': '302ff5394749f6d0d041ec0744db1ec29d06f46a22e4fd72973c6585b112ca79',
    'vendor/glib/src/value_array.rs': '8509fd706cbd4f2dffb75f2001ce4365350ab16f54f555bd72d3c607381d96e5',
    'vendor/glib/src/variant.rs': 'fe31f692d591a4c3a1270f2f19a2101fa73725cdf9bee004478639d1ec7ebd8b',
    'vendor/glib/src/variant_iter.rs': 'a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc',
    'vendor/glib/src/variant_type.rs': '5b0be1eab8effe786282118af55c134a681e10406b6fe5e8ba598a9a46016847',
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
