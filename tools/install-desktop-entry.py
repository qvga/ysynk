#!/usr/bin/env python3
"""Register a built checkout in the current user's Linux application menu."""
import argparse
import errno
import os
from pathlib import Path
import stat
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def desktop_value(value):
    if any(ord(character) < 32 for character in value):
        raise ValueError('Desktop entry paths must not contain control characters')
    return value.replace('\\', '\\\\')


def executable_value(path):
    text = str(path)
    if any(character in text for character in '=%'):
        raise ValueError('Desktop entry executable paths must not contain equals or percent signs')
    # Desktop string escapes are decoded before command-line quoting. This is
    # not a shell command; shell quoting (such as shlex.quote) is not valid here.
    quoted = ''.join('\\' + character if character in '\\"`$' else character
                     for character in text)
    return desktop_value('"' + quoted + '"')


def check_existing_entry(path, marker):
    # Desktop launchers are plain files. Inspect the opened object so a FIFO
    # cannot stall registration, and a symlink cannot select a different file.
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return
    except OSError as error:
        if error.errno == errno.ELOOP:
            raise ValueError('Refusing to replace a desktop entry symlink') from error
        raise
    with os.fdopen(fd, 'rb') as entry:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise ValueError(f'Desktop entry must be a regular file: {path}')
        expected = marker.encode('utf-8')
        if not any(line.rstrip(b'\r\n') == expected for line in entry):
            raise ValueError(f'Refusing to replace an unmanaged desktop entry: {path}')


def install(executable, data_home):
    executable = executable.resolve(strict=True)
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError('Build the desktop executable before registering it')
    if not data_home.is_absolute():
        raise ValueError('XDG_DATA_HOME must be an absolute path')
    icon = ROOT / 'desktop/icons/128x128.png'
    if not icon.is_file():
        raise ValueError('The checkout application icon is missing')
    marker = 'X-ysynk-Checkout=' + desktop_value(str(ROOT))
    destination = data_home / 'applications/ysynk.desktop'
    check_existing_entry(destination, marker)
    replacements = {'Exec': executable_value(executable), 'Icon': desktop_value(str(icon))}
    lines = []
    for line in (ROOT / 'packaging/ysynk.desktop').read_text().splitlines():
        key = line.partition('=')[0]
        lines.append(key + '=' + replacements[key] if key in replacements else line)
    entry = '\n'.join(lines)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=destination.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(entry + '\n' + marker + '\n')
            output.close()
            temporary.chmod(0o644)
            temporary.replace(destination)
        finally:
            temporary.unlink(missing_ok=True)
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path, nargs='?',
                        default=ROOT / 'target/debug/ysynk-desktop')
    args = parser.parse_args()
    data_home = Path(os.environ.get('XDG_DATA_HOME') or Path.home() / '.local/share')
    try:
        destination = install(args.executable, data_home)
    except (OSError, ValueError) as error:
        parser.exit(1, f'{error}\n')
    print(f'Registered {destination}. Reopen ysynk to refresh its taskbar icon.')


if __name__ == '__main__':
    main()
