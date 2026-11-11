#!/usr/bin/env python3
"""Build a native multiplayer client + dedicated server ZIP on macOS, Windows or Linux.
Run on each destination OS; this is not a cross compiler. Converted assets are optional.
"""
import argparse
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, help='Include converted assets for the chosen track/car')
    parser.add_argument('--car', choices=['rb6', 'gen22', 'gen26'], default='rb6')
    parser.add_argument('--track', default='spa')
    parser.add_argument('--skip-build', action='store_true')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--workspace'], cwd=root, check=True)
    system = platform.system().lower()
    suffix = '.exe' if system == 'windows' else ''
    output = root / 'dist'
    output.mkdir(exist_ok=True)
    name = f'F1-Racecraft-multiplayer-{system}-{platform.machine().lower()}'
    target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'target'))
    if not target.is_absolute():
        target = root / target
    with tempfile.TemporaryDirectory(prefix='racecraft-package-') as tmp:
        stage = Path(tmp) / name
        stage.mkdir()
        for binary in ['f1-the-game', 'racecraft-server']:
            shutil.copy2(target / 'release' / f'{binary}{suffix}', stage)
        if args.assets:
            assets = args.assets.resolve()
            for part in [f'tracks/{args.track}', f'cars/{args.car}']:
                shutil.copytree(assets / part, stage / 'assets' / part)
        shutil.copy2(root / 'docs' / 'multiplayer.md', stage / 'MULTIPLAYER.md')
        if system == 'windows':
            (stage / 'Play.bat').write_text(f'@echo off\r\ncd /d "%~dp0"\r\nf1-the-game.exe --assets assets --car {args.car} %*\r\n', encoding='utf-8')
        else:
            launcher = stage / ('Play.command' if system == 'darwin' else 'Play.sh')
            launcher.write_text(f'#!/bin/sh\ncd "$(dirname "$0")" || exit 1\nexec ./f1-the-game --assets ./assets --car {args.car} "$@"\n')
            launcher.chmod(0o755)
        if not args.assets:
            (stage / 'ASSETS_REQUIRED.txt').write_text('Extract the matching converted assets into ./assets before playing. See MULTIPLAYER.md.\n')
        archive = output / f'{name}.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as zip_out:
            for file in sorted(stage.rglob('*')):
                if file.is_file():
                    zip_out.write(file, file.relative_to(stage.parent))
        print(archive)


if __name__ == '__main__':
    main()
