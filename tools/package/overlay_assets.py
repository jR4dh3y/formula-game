"""Overlay a pinned asset add-on archive (for example team liveries) onto extracted game assets.

Each top-level directory below `--replace` in the add-on replaces the matching base
directory, so stale files from the base archive never mix with new artwork.
"""
import argparse
import shutil
import tempfile
from pathlib import Path

from extract_assets import extract


def overlay(archive: Path, assets: Path, sha256: str, replace: str) -> list[str]:
    with tempfile.TemporaryDirectory() as temporary:
        staged = Path(temporary) / "assets"
        extract(archive, staged, sha256)
        source = staged / replace
        if not source.is_dir():
            raise ValueError(f"Add-on archive has no {replace}")
        target = assets / replace
        target.mkdir(parents=True, exist_ok=True)
        replaced = []
        for item in sorted(source.iterdir()):
            destination = target / item.name
            if destination.is_dir():
                shutil.rmtree(destination)
            elif destination.exists():
                destination.unlink()
            if item.is_dir():
                shutil.copytree(item, destination)
            else:
                shutil.copy2(item, destination)
            replaced.append(item.name)
        return replaced


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("assets", type=Path, help="extracted assets directory to update")
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--replace", required=True, help="path below assets, e.g. cars/gen22/liveries")
    args = parser.parse_args()
    names = overlay(args.archive, args.assets, args.sha256, args.replace)
    print(f"Overlaid {len(names)} entries into {args.assets / args.replace}: {', '.join(names)}")


if __name__ == "__main__":
    main()
