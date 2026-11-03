import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location('advisories', Path(__file__).with_name('check_advisories.py'))
advisories = importlib.util.module_from_spec(spec)
spec.loader.exec_module(advisories)


class AdvisoryInventoryTests(unittest.TestCase):
    def scan(self, root, packages, query):
        (root / 'Cargo.lock').write_text('version = 4\npackage = []\n')
        (root / 'package-lock.json').write_text(json.dumps(packages))
        with patch.object(advisories, '__file__', str(root / 'tools/check_advisories.py')), \
                patch.object(advisories, 'query', side_effect=query), \
                patch('sys.argv', ['check_advisories.py']), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            code = advisories.main()
        return code, json.loads(output.getvalue())

    def test_local_links_are_queried_once_with_the_upstream_package_name(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            vendor = root / 'vendor/deepmerge'
            vendor.mkdir(parents=True)
            (vendor / 'package.json').write_text(json.dumps({'name': 'deepmerge', 'version': '4.3.1'}))
            queries = []
            def query(batch):
                queries.extend(batch)
                return [{} for _ in batch]
            code, report = self.scan(root, {'lockfileVersion': 3, 'packages': {
                '': {'name': 'application', 'version': '1'},
                'node_modules/deepmerge': {'link': True, 'resolved': 'vendor/deepmerge'},
                'vendor/deepmerge': {'version': '4.3.1'},
            }}, query)
            self.assertEqual(code, 0)
            self.assertEqual(report['packages_checked'], 1)
            self.assertEqual(queries, [{'package': {'name': 'deepmerge', 'ecosystem': 'npm'}, 'version': '4.3.1'}])

    def test_aliases_are_queried_by_the_declared_package_name(self):
        with tempfile.TemporaryDirectory() as directory:
            queries = []
            def query(batch):
                queries.extend(batch)
                return [{} for _ in batch]
            self.scan(Path(directory), {'lockfileVersion': 3, 'packages': {
                '': {},
                'node_modules/alias': {'name': 'upstream-package', 'version': '1.2.3'},
                'node_modules/parent/node_modules/@scope/child': {'version': '2.3.4'},
            }}, query)
            self.assertEqual({q['package']['name'] for q in queries}, {'upstream-package', '@scope/child'})

    def test_missing_package_inventory_does_not_produce_a_clean_report(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'npm lockfile'):
                self.scan(Path(directory), {'lockfileVersion': 3}, lambda batch: [{} for _ in batch])

    def test_incomplete_and_paginated_responses_fail_the_check(self):
        with tempfile.TemporaryDirectory() as directory:
            lock = {'lockfileVersion': 3, 'packages': {'': {}, 'node_modules/example': {'version': '1'}}}
            for response in ([], [{'next_page_token': 'more'}]):
                with self.subTest(response=response), self.assertRaisesRegex(RuntimeError, 'Incomplete'):
                    self.scan(Path(directory), lock, lambda batch: response)

    def test_advisories_are_retained_and_return_a_failure_status(self):
        with tempfile.TemporaryDirectory() as directory:
            code, report = self.scan(Path(directory), {
                'lockfileVersion': 3, 'packages': {'': {}, 'node_modules/example': {'version': '1'}},
            }, lambda batch: [{'vulns': [{'id': 'EXAMPLE-1'}]}])
            self.assertEqual(code, 1)
            self.assertEqual(report['findings'], [{
                'package': 'example', 'version': '1', 'ecosystem': 'npm', 'advisories': ['EXAMPLE-1'],
            }])
