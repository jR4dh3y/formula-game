#!/usr/bin/env python3
"""Build a single-file Windows game with bundled assets and no installer."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import tomllib
import zipfile
import zlib

ROOT = Path(__file__).resolve().parents[2]
LAUNCHER = ROOT / "tools/package/windows_launcher/Cargo.toml"
TARGET = "x86_64-pc-windows-gnu"
FOOTER = b"F1RACECRAFT-PKG1"


def cargo_output(cargo, *args):
    return subprocess.check_output(cargo + list(args), cwd=ROOT, text=True)


def copy_notices(cargo, manifest, destination):
    metadata = json.loads(cargo_output(
        cargo, "metadata", "--locked", "--manifest-path", str(manifest),
        "--format-version", "1"))
    notices = []
    for package in metadata["packages"]:
        notices.append({key: package.get(key) for key in
                        ("name", "version", "license", "repository", "source")})
        if not package.get("source"):
            continue
        for source in Path(package["manifest_path"]).parent.iterdir():
            if source.is_file() and source.name.upper().startswith(
                    ("LICENSE", "LICENCE", "COPYING", "NOTICE")):
                folder = destination / "licenses/rust" / f'{package["name"]}-{package["version"]}'
                folder.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, folder / source.name)
    return notices


def write_single_exe(stub, directory, output):
    """Append a zlib-compressed, length-delimited file stream to the PE launcher.

    The final 24 bytes contain the magic and compressed length. Decompressed:
    F1ASSET1 + u32 file count, then records of u32 UTF-8 path length, u64 file
    size, path bytes, file bytes (little endian). Paths use '/'. Rust rejects
    traversal/duplicates and checks file lengths and the zlib checksum.
    """
    files = sorted(path for path in directory.rglob("*") if path.is_file())
    temporary = output.with_suffix(".exe.tmp")
    compressor = zlib.compressobj(level=6)
    with stub.open("rb") as source, temporary.open("wb") as bundle:
        shutil.copyfileobj(source, bundle)
        start = bundle.tell()
        bundle.write(compressor.compress(b"F1ASSET1" + struct.pack("<I", len(files))))
        for source in files:
            name = source.relative_to(directory).as_posix().encode("utf-8")
            bundle.write(compressor.compress(struct.pack("<IQ", len(name), source.stat().st_size)))
            bundle.write(compressor.compress(name))
            with source.open("rb") as content:
                while block := content.read(1024 * 1024):
                    bundle.write(compressor.compress(block))
        bundle.write(compressor.flush())
        compressed_size = bundle.tell() - start
        bundle.write(FOOTER + struct.pack("<Q", compressed_size))
    temporary.replace(output)


def write_checksum(path):
    with path.open("rb") as content:
        digest = hashlib.file_digest(content, "sha256").hexdigest()
    Path(str(path) + ".sha256").write_text(f"{digest}  {path.name}\n", encoding="ascii")
    print(f"{path}\nSHA256: {digest}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true", help="reuse the game EXE; still build the launcher")
    parser.add_argument("--target", choices=[TARGET], default=TARGET)
    parser.add_argument("--cargo-config", help="optional local Cargo configuration file")
    parser.add_argument("--zip", action="store_true", help="also create a traditional unpacked-folder ZIP")
    args = parser.parse_args()
    cargo = ["cargo"] + (["--config", args.cargo_config] if args.cargo_config else [])
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    assets = Path(os.environ.get("F1_PACKAGE_ASSETS", ROOT / "assets")).resolve()
    required = [
        *(Path("cars") / car / name for car in ("rb6", "rb22")
          for name in ("car.json", "car.glb")),
        *(Path("tracks/spa") / name for name in ("track.json", "collision.bin", "visual.glb")),
    ]
    for name in required:
        if not (assets / name).is_file():
            raise FileNotFoundError(assets / name)
    if not args.skip_build:
        subprocess.run(cargo + ["build", "--release", "--locked", "-p",
                        "f1-the-game", "--target", args.target], cwd=ROOT, check=True)
    binary = ROOT / "target" / args.target / "release/f1-the-game.exe"
    if not binary.is_file():
        raise FileNotFoundError(binary)
    subprocess.run(cargo + ["build", "--release", "--locked", "--manifest-path", str(LAUNCHER),
                           "--target", args.target, "--target-dir", str(ROOT / "target/windows-launcher")],
                   cwd=ROOT, check=True)
    stub = ROOT / "target/windows-launcher" / args.target / "release/racecraft-windows-launcher.exe"
    work = ROOT / "target/packaging"
    work.mkdir(parents=True, exist_ok=True)
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    name = f"F1-Racecraft-{version}-windows-x86_64"
    with tempfile.TemporaryDirectory(prefix="windows-", dir=work) as temporary:
        package = Path(temporary) / name
        package.mkdir()
        shutil.copy2(binary, package / "f1-the-game.exe")
        for path in required:
            destination = package / "assets" / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(assets / path, destination)
        (package / "Play.cmd").write_text(
            '@echo off\ncd /d "%~dp0"\n'
            'if not defined F1_SETTINGS set "F1_SETTINGS=%LOCALAPPDATA%\\f1-racecraft\\preferences.json"\n'
            'if not defined XDG_CACHE_HOME set "XDG_CACHE_HOME=%LOCALAPPDATA%\\f1-racecraft"\n'
            '"%~dp0f1-the-game.exe" --assets "%~dp0assets" %*\n'
            'if errorlevel 1 pause\n', encoding="utf-8")
        shutil.copy2(ROOT / "tools/package/WINDOWS.md", package / "README.md")
        licenses = package / "licenses"
        licenses.mkdir()
        shutil.copy2(ROOT / "game/fonts/OFL.txt", licenses / "FONT-OFL.txt")
        shutil.copy2(ROOT / "game/audio/reference/CREDITS.md", licenses / "AUDIO-CREDITS.md")
        notices = copy_notices(cargo, ROOT / "Cargo.toml", package)
        notices += copy_notices(cargo, LAUNCHER, package)
        notices = {(p["name"], p["version"]): p for p in notices}
        (package / "RUST-PACKAGES.json").write_text(
            json.dumps(list(notices.values()), indent=2), encoding="utf-8")
        (package / "BUILD.json").write_text(json.dumps({
            "source": "https://github.com/jR4dh3y/formula-game",
            "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "target": args.target, "experimental": False, "tracks": ["spa"],
            "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
            "assets_sha256": {path.as_posix(): hashlib.sha256((assets / path).read_bytes()).hexdigest()
                              for path in required},
        }, indent=2), encoding="utf-8")
        executable = dist / f"{name}.exe"
        write_single_exe(stub, package, executable)
        write_checksum(executable)
        if args.zip:
            archive = dist / f"{name}.zip"
            pending = archive.with_suffix(".zip.tmp")
            with zipfile.ZipFile(pending, "w", compression=zipfile.ZIP_DEFLATED,
                                 strict_timestamps=False) as bundle:
                for source in sorted(package.rglob("*")):
                    if source.is_file():
                        bundle.write(source, source.relative_to(package.parent))
            pending.replace(archive)
            write_checksum(archive)


if __name__ == "__main__":
    main()
