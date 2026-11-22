import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location('notices', Path(__file__).with_name('release-notices.py'))
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class ReleaseNoticeTests(unittest.TestCase):
    def test_collects_exact_root_notices_and_declared_contained_license(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'LICENSE-MIT').write_bytes(b'copyright\r\npermission\r\n')
            (root / 'NOTICE').write_text('attribution\n')
            (root / 'legal').mkdir()
            (root / 'legal/terms.txt').write_text('declared license\n')
            (root / 'fixtures').mkdir()
            (root / 'fixtures/LICENSE').write_text('not distributed\n')
            self.assertEqual(notices.collect_files(root, 'legal/terms.txt'), {
                'LICENSE-MIT': 'copyright\r\npermission\r\n',
                'NOTICE': 'attribution\n',
                'legal/terms.txt': 'declared license\n',
            })
            self.assertIn('declared license\n', notices.section(
                'crate', '1', 'MIT', [], {'legal/terms.txt': 'declared license\n'},
                has_declared_license=True))

    def test_rejects_license_paths_and_symlinks_outside_package(self):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / 'crate'
            root.mkdir()
            (parent / 'secret').write_text('not a package license')
            with self.assertRaisesRegex(ValueError, 'escapes package root'):
                notices.collect_files(root, '../secret')
            (root / 'LICENSE').symlink_to(parent / 'secret')
            with self.assertRaisesRegex(ValueError, 'escapes package root'):
                notices.collect_files(root)

    def test_rejects_missing_empty_and_copyright_only_licenses(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(FileNotFoundError):
                notices.collect_files(root, 'missing.txt')
            (root / 'LICENSE').write_text(' ')
            with self.assertRaisesRegex(ValueError, 'empty license'):
                notices.collect_files(root)
        with self.assertRaisesRegex(ValueError, 'no license text'):
            notices.section('crate', '1', 'MIT', [], {'COPYRIGHT': 'author'})

    def test_missing_upstream_material_does_not_access_network_offline(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = {'name': 'example', 'version': '1', 'manifest_path': str(root / 'Cargo.toml')}
            with patch.object(notices, 'fetch_json', side_effect=AssertionError('network')):
                with self.assertRaisesRegex(ValueError, 'prepare cache'):
                    notices.supplement(package, 'https://github.com/example/repo', 'a' * 40, root, False)

    def test_cached_upstream_notice_requires_matching_content_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / 'cache'
            package = {'name': 'example', 'version': '1', 'manifest_path': str(root / 'Cargo.toml')}
            entries = [{'name': 'LICENSE', 'path': 'LICENSE', 'type': 'file'}]
            arguments = (package, 'https://github.com/example/repo', 'a' * 40, cache)
            with patch.object(notices, 'fetch_json', return_value=entries), \
                    patch.object(notices, 'urlopen', return_value=io.BytesIO(b'original license\n')):
                expected = notices.supplement(*arguments, True)
            self.assertEqual(notices.supplement(*arguments, False), expected)
            cached_file = next(cache.glob('*.json'))
            data = json.loads(cached_file.read_text())
            data['files'][0]['text'] = 'changed license'
            cached_file.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, 'invalid upstream notice cache file'):
                notices.supplement(*arguments, False)

    def test_bundle_excludes_workspace_and_preserves_license_expression(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for path in ('node_modules/svelte', 'node_modules/@tauri-apps/api', 'vendor/deepmerge',
                         'node_modules/clsx', 'node_modules/esm-env',
                         'node_modules/vite', 'node_modules/rolldown'):
                package_root = root / path
                package_root.mkdir(parents=True)
                (package_root / 'LICENSE').write_text('exact JavaScript terms\n')
                (package_root / 'package.json').write_text(json.dumps({
                    'name': path.split('/')[-1], 'version': '1', 'license': 'MIT',
                    'repository': 'example/clsx' if path.endswith('/clsx') else {'url': 'https://github.com/example/repo'},
                }))
            (root / 'package.json').write_text('{"version":"1.2.3"}')
            (root / 'LICENSE').write_text('exact Rust terms\n')
            metadata = {
                'workspace_members': ['own'],
                'resolve': {'nodes': [{'id': 'own'}, {'id': 'dep'}]},
                'packages': [
                    {'id': 'own', 'name': 'own', 'version': '1', 'license': None},
                    {'id': 'dep', 'name': 'dep', 'version': '1', 'license': 'Apache-2.0 OR GPL-2.0-only',
                     'source': 'registry+https://github.com/rust-lang/crates.io-index',
                     'manifest_path': str(root / 'Cargo.toml')},
                ],
            }
            with patch.object(notices, 'ROOT', root):
                first = notices.generate(metadata, root / 'cache', False)
                self.assertEqual(first, notices.generate(metadata, root / 'cache', False))
                self.assertTrue(first.startswith('ysynk 1.2.3 - third-party notices'))
                self.assertIn('Inventory: 1 Rust packages; 7 JavaScript packages.', first)
                self.assertIn('\nclsx 1\n', first)
                self.assertIn('\nesm-env 1\n', first)
                self.assertIn('Declared license: Apache-2.0 OR GPL-2.0-only', first)
                self.assertIn('Selected license: Apache-2.0 (not GPL-2.0-only)', first)
                self.assertIn('exact Rust terms\n', first)
                self.assertNotIn(str(root), first)
                self.assertNotIn('\nown 1\n', first)
                (root / 'LICENSE').unlink()
                with self.assertRaisesRegex(ValueError, 'Required third-party notices unavailable'):
                    notices.generate(metadata, root / 'cache', False)

    def test_rejects_credentials_and_non_public_source_urls(self):
        for url in ('file:///home/private', 'https://user:secret@github.com/example/repo',
                    'https://github.com/example/repo?token=secret'):
            with self.assertRaises(ValueError):
                notices.public_url(url)
