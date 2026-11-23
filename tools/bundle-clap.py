#!/usr/bin/env python3
"""Package an already-built CLAP library; no signing or cross-compilation."""
import argparse
import pathlib
import plistlib
import shutil
import sys

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--library', type=pathlib.Path, required=True)
parser.add_argument('--name', default='SoniclawDemo')
parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path('dist'))
parser.add_argument('--platform', choices=['linux', 'windows', 'macos'], default='macos' if sys.platform == 'darwin' else 'windows' if sys.platform == 'win32' else 'linux')
parser.add_argument('--identifier', default='org.soniclaw.demo')
args = parser.parse_args()
if not args.library.is_file():
    parser.error('build the shared library first, then pass its path')
if not args.name or args.name in {'.', '..'} or any(c in args.name for c in '/\\'):
    parser.error('name must be a plain filename')
args.output.mkdir(parents=True, exist_ok=True)
destination = args.output / (args.name + '.clap')
if destination.exists():
    parser.error(f'{destination} already exists; choose another output or remove it explicitly')
if args.platform == 'macos':
    binary = destination / 'Contents' / 'MacOS' / args.name
    binary.parent.mkdir(parents=True)
    shutil.copy2(args.library, binary)
    binary.chmod(binary.stat().st_mode | 0o111)
    with (destination / 'Contents' / 'Info.plist').open('wb') as file:
        plistlib.dump({'CFBundleName': args.name, 'CFBundleExecutable': args.name, 'CFBundleIdentifier': args.identifier,
                      'CFBundlePackageType': 'BNDL', 'CFBundleVersion': '0.146.0', 'CFBundleShortVersionString': '0.146.0'}, file)
else:
    shutil.copy2(args.library, destination)
print(destination.resolve())
