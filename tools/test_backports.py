import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('backports', Path(__file__).with_name('check-backports.py'))
backports = importlib.util.module_from_spec(spec)
spec.loader.exec_module(backports)


class BackportSourceTests(unittest.TestCase):
    def test_accepts_only_the_reviewed_file_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.write_bytes(b'reviewed source')
            expected = {'source': hashlib.sha256(source.read_bytes()).hexdigest()}
            backports.verify_files(root, expected)
            source.write_bytes(b'changed source')
            with self.assertRaises(ValueError):
                backports.verify_files(root, expected)

    def test_rejects_missing_source_and_symlink_replacements(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ValueError):
                backports.verify_files(root, {'source': 'unused'})
            (root / 'real').write_bytes(b'source')
            (root / 'source').symlink_to('real')
            with self.assertRaises(ValueError):
                backports.verify_files(root, {'source': hashlib.sha256(b'source').hexdigest()})
