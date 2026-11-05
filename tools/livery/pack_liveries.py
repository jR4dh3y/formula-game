"""Pack assets/cars/gen22/liveries into a reproducible add-on ZIP and print its SHA-256.

Same input always gives the same bytes, so the hash can be pinned in CI:
    python tools/livery/pack_liveries.py --assets assets --output f1-racecraft-liveries.zip
"""
import argparse
import hashlib
import zipfile
from pathlib import Path


def pack(assets: Path, output: Path) -> str:
    source = assets / "cars/gen22/liveries"
    files = sorted(path for path in source.rglob("*") if path.is_file())
    if not files:
        raise SystemExit(f"No liveries under {source}")
    with zipfile.ZipFile(output, "w", zipfile.ZIP_STORED) as archive:
        for path in files:
            info = zipfile.ZipInfo("assets/" + path.relative_to(assets).as_posix(), date_time=(2026, 11, 5, 0, 0, 0))
            info.external_attr = 0o644 << 16
            archive.writestr(info, path.read_bytes())
    return hashlib.sha256(output.read_bytes()).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, default=Path("assets"))
    parser.add_argument("--output", type=Path, default=Path("f1-racecraft-liveries.zip"))
    args = parser.parse_args()
    digest = pack(args.assets.resolve(), args.output)
    print(f"{digest}  {args.output.name}")
    print(f"release tag: liveries-{digest[:16]}")


if __name__ == "__main__":
    main()
