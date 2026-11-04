import importlib.util
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    'desktop_entry', Path(__file__).with_name('install-desktop-entry.py'))
desktop_entry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(desktop_entry)


class DesktopEntryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.executable = self.root / 'build with spaces/ysynk-desktop'
        self.executable.parent.mkdir()
        self.executable.write_text('#!/bin/sh\nexit 0\n')
        self.executable.chmod(0o755)
        self.data_home = self.root / 'share'

    def test_registers_the_built_executable_and_checkout_icon(self):
        destination = desktop_entry.install(self.executable, self.data_home)
        entry = destination.read_text()
        self.assertEqual(destination, self.data_home / 'applications/ysynk.desktop')
        self.assertIn(f'Exec="{self.executable}"\n', entry)
        self.assertIn(f'Icon={desktop_entry.ROOT}/desktop/icons/128x128.png\n', entry)
        self.assertIn('StartupWMClass=ysynk-desktop\n', entry)
        self.assertIn('Terminal=false\n', entry)

    def test_repeated_registration_updates_the_managed_launcher(self):
        destination = desktop_entry.install(self.executable, self.data_home)
        release = self.root / 'release/ysynk-desktop'
        release.parent.mkdir()
        release.write_bytes(self.executable.read_bytes())
        release.chmod(0o755)
        desktop_entry.install(release, self.data_home)
        self.assertIn(f'Exec="{release}"\n', destination.read_text())
        self.assertNotIn(str(self.executable), destination.read_text())

    def test_existing_unmanaged_launchers_are_not_replaced(self):
        destination = self.data_home / 'applications/ysynk.desktop'
        destination.parent.mkdir(parents=True)
        original = '[Desktop Entry]\nName=Custom ysynk\n'
        destination.write_text(original)
        with self.assertRaisesRegex(ValueError, 'unmanaged desktop entry'):
            desktop_entry.install(self.executable, self.data_home)
        self.assertEqual(destination.read_text(), original)

    def test_desktop_entry_symlinks_are_not_replaced(self):
        destination = self.data_home / 'applications/ysynk.desktop'
        destination.parent.mkdir(parents=True)
        destination.symlink_to(self.root / 'missing')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            desktop_entry.install(self.executable, self.data_home)
        self.assertTrue(destination.is_symlink())

    def test_invalid_executables_and_data_directories_do_not_create_launchers(self):
        with self.assertRaises(FileNotFoundError):
            desktop_entry.install(self.root / 'missing', self.data_home)
        self.executable.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'Build the desktop executable'):
            desktop_entry.install(self.executable, self.data_home)
        self.executable.chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'absolute path'):
            desktop_entry.install(self.executable, Path('relative'))
        self.assertFalse(self.data_home.exists())

    def test_executable_paths_use_desktop_entry_escaping_not_shell_quoting(self):
        self.assertEqual(desktop_entry.executable_value(Path('/tmp/a b/ysynk-desktop')),
                         '"/tmp/a b/ysynk-desktop"')
        # Both desktop-string decoding and Exec argument decoding consume a
        # backslash. These characters must still reach the executable unchanged.
        self.assertEqual(desktop_entry.executable_value(Path('/tmp/$`"\\')),
                         '"/tmp/' + '\\\\$' + '\\\\`' + '\\\\"' + '\\\\\\\\' + '"')
        for path in ('/tmp/a=b', '/tmp/a%fb', '/tmp/a\nb', '/tmp/a\rb', '/tmp/a\tb'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                desktop_entry.executable_value(Path(path))
