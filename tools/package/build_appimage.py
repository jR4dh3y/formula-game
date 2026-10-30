#!/usr/bin/env python3
"""Build a portable Linux x86-64 AppImage, including baseline (non-v3) libraries.

Build host: Arch-compatible Linux with cargo, gcc binutils, pacman/keyring,
Python, curl, tar/zstd and mksquashfs. Players need none of these tools.
Official Arch package signatures and the pinned runtime digest are verified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import urllib.request
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "target/packaging"
CACHE = WORK / "downloads"
APP = WORK / "F1-Racecraft.AppDir"
DIST = ROOT / "dist"
RUNTIME_URL = "https://github.com/VHSgunzo/uruntime/releases/download/v0.8.1/uruntime-appimage-squashfs-lite-x86_64"
RUNTIME_SHA = "32c6024a14ff375749ee3bed3330c1586a6dd2bfeddbbd7ceacc053aec2b27a3"
PACKAGES = {}


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs)


def fetch(url, path):
    if not path.exists():
        temporary = path.with_suffix(path.suffix + ".download")
        subprocess.run(["curl", "--connect-timeout", "15", "--max-time", "180", "--retry", "2",
                        "-fLsS", url, "-o", str(temporary)], check=True)
        temporary.replace(path)
    return path


def package(name):
    if name in PACKAGES:
        return PACKAGES[name][0]
    metadata_file = CACHE / f"{name}.json"
    fetch("https://archlinux.org/packages/search/json/?" + urllib.parse.urlencode({"name": name}), metadata_file)
    matches = json.loads(metadata_file.read_text())["results"]
    meta = next(p for p in matches if p["pkgname"] == name and p["arch"] in ("x86_64", "any")
                and p["repo"] in ("core", "extra"))
    url = f'https://archlinux.org/packages/{meta["repo"]}/{meta["arch"]}/{name}/download/'
    archive = fetch(url, CACHE / meta["filename"])
    # The redirect target has a sibling detached signature.
    signature = CACHE / (meta["filename"] + ".sig")
    if not signature.exists():
        with urllib.request.urlopen(url, timeout=30) as response:
            fetch(response.geturl() + ".sig", signature)
    keyring = CACHE / "archlinux-keyring.gpg"
    if not keyring.exists():
        with keyring.open("wb") as output:
            subprocess.run(["gpg", "--batch", "--dearmor", "--output", "-", "/usr/share/pacman/keyrings/archlinux.gpg"],
                           stdout=output, check=True)
    result = subprocess.run(["gpgv", "--keyring", str(keyring),
                             str(signature), str(archive)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"Signature verification failed for {name}: {result.stderr}")
    extracted = WORK / "packages" / name
    if not extracted.exists():
        extracted.mkdir(parents=True)
        subprocess.run(["tar", "-xf", str(archive), "-C", str(extracted)], check=True)
    meta["sha256"] = hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()
    meta["download"] = url
    PACKAGES[name] = (extracted, meta)
    print(f"Verified baseline library package: {name}", flush=True)
    return extracted


def find_library(name):
    for root, _ in PACKAGES.values():
        for candidate in root.glob(f"usr/lib/**/{name}"):
            # Resolve absolute package symlinks inside its root, never on the host.
            for _ in range(16):
                if not candidate.is_symlink():
                    if candidate.is_file():
                        return candidate
                    break
                target = Path(os.readlink(candidate))
                candidate = root / str(target).lstrip("/") if target.is_absolute() else candidate.parent / target
    return None


def owner(name):
    candidates = list(Path("/usr/lib").glob(f"**/{name}"))
    if not candidates:
        raise RuntimeError(f"Cannot find build-host provider of {name}")
    return run("pacman", "-Qqo", str(candidates[0].resolve())).strip()


def needed(path):
    return re.findall(r"\(NEEDED\).*\[(.*?)\]", run("readelf", "-d", str(path)))


def validate_gen22_assets(directory):
    """Reject incomplete car and roster texture trees before packaging them."""
    for name in ("car.glb", "car_lod1.glb", "car.json", "teams.json"):
        path = directory / name
        if not path.is_file():
            raise RuntimeError(f"Missing GEN22 asset: {path}")
    roster = json.loads((directory / "teams.json").read_text())
    for team in roster["teams"]:
        paths = [Path("paint.png"), Path("suit.png")]
        paths += [Path(driver["id"]) / "helmet.png" for driver in team["drivers"]]
        for relative in paths:
            path = directory / "liveries" / team["id"] / relative
            if not path.is_file():
                raise RuntimeError(f"Missing GEN22 livery texture: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exp", action="store_true", help="build with experimental circuits enabled")
    args = parser.parse_args()
    assets = Path(os.environ.get("F1_PACKAGE_ASSETS", str(ROOT / "assets"))).resolve()
    validate_gen22_assets(assets / "cars/gen22")
    tracks = ["spa", "silverstone", "bahrain", "cota"] if args.exp else ["spa"]
    CACHE.mkdir(parents=True, exist_ok=True)
    DIST.mkdir(exist_ok=True)
    build = ["cargo", "build", "--release", "--locked", "-p", "f1-the-game"]
    if args.exp:
        build += ["--features", "exp"]
    subprocess.run(build, cwd=ROOT, check=True)
    if APP.exists():
        shutil.rmtree(APP)
    for directory in ["usr/bin", "usr/lib", "usr/share/f1-racecraft/assets", "usr/share/licenses/f1-racecraft"]:
        (APP / directory).mkdir(parents=True)
    binary = APP / "usr/bin/f1-the-game"
    shutil.copy2(ROOT / "target/release/f1-the-game", binary)
    subprocess.run(["strip", "--strip-unneeded", str(binary)], check=True)
    shutil.copy2(ROOT / "tools/package/AppRun", APP / "AppRun")
    (APP / "AppRun").chmod(0o755)
    for category, ids, files in [
        ("tracks", tracks, ["track.json", "collision.bin", "visual.glb"]),
    ]:
        for item in ids:
            destination = APP / "usr/share/f1-racecraft/assets" / category / item
            destination.mkdir(parents=True)
            for filename in files:
                shutil.copy2(assets / category / item / filename, destination / filename)

    # Include the full modern livery/helmet tree and every legacy asset.
    for car in ["gen22", "rb6"]:
        shutil.copytree(assets / "cars" / car,
                        APP / "usr/share/f1-racecraft/assets/cars" / car)

    # Only generic x86-64 official packages are used. The build host's CachyOS
    # AVX2/v3-optimized libraries must not impose that CPU requirement on players.
    seeds = ["libvulkan.so.1", "libwayland-client.so.0", "libwayland-cursor.so.0", "libwayland-egl.so.1",
             "libxkbcommon.so.0", "libxkbcommon-x11.so.0", "libX11.so.6", "libXcursor.so.1",
             "libXrandr.so.2", "libXi.so.6", "libXinerama.so.1", "libXrender.so.1", "libXfixes.so.3",
             "libXext.so.6", "libxcb.so.1", "libstdc++.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1",
             "libasound_module_pcm_pulse.so", "libasound_module_ctl_pulse.so", "libasound_module_conf_pulse.so",
             "ld-linux-x86-64.so.2"]
    queue = needed(binary) + seeds
    copied = set()
    while queue:
        name = queue.pop()
        if name in copied:
            continue
        source = find_library(name)
        if source is None:
            package(owner(name))
            source = find_library(name)
        if source is None:
            raise RuntimeError(f"Baseline package is missing {name}")
        shutil.copy2(source, APP / "usr/lib" / name)
        copied.add(name)
        queue.extend(needed(source))

    alsa = package("alsa-lib") / "usr/share/alsa"
    shutil.copytree(alsa, APP / "usr/share/alsa", symlinks=False)
    # PulseAudio also works with PipeWire's Pulse server; hardware ALSA remains fallback.
    config = APP / "usr/share/alsa/alsa.conf"
    # Do not import host plugin definitions for libraries outside this bundle.
    base = "# load card-specific" + config.read_text().split("# load card-specific", 1)[1]
    config.write_text(base + '\npcm.!default { type pulse fallback "sysdefault" }\nctl.!default { type pulse fallback "sysdefault" }\n')
    xkb = package("xkeyboard-config") / "usr/share"
    keyboard = next(p for p in xkb.glob("xkeyboard-config*") if p.is_dir())
    shutil.copytree(keyboard, APP / "usr/share/X11/xkb", symlinks=False)
    licenses = APP / "usr/share/licenses"
    for name, (path, _) in PACKAGES.items():
        source = path / "usr/share/licenses"
        if source.exists():
            shutil.copytree(source, licenses / "system" / name, dirs_exist_ok=True, symlinks=False)
    # License texts for system packages whose distro metadata uses SPDX names.
    for license_id in ["LGPL-2.1-or-later", "GPL-2.0-or-later", "GPL-3.0-or-later", "GCC-exception-3.1"]:
        source = Path("/usr/share/licenses/spdx") / f"{license_id}.txt"
        if not source.exists():
            source = Path("/usr/share/licenses/spdx/exceptions") / f"{license_id}.txt"
        if source.exists():
            shutil.copy2(source, licenses / source.name)
    shutil.copy2(ROOT / "game/fonts/OFL.txt", licenses / "f1-racecraft/FONT-OFL.txt")
    shutil.copy2(ROOT / "game/audio/reference/CREDITS.md", licenses / "f1-racecraft/AUDIO-CREDITS.md")
    shutil.copy2(ROOT / "tools/package/README.md", APP / "README.txt")
    (APP / "BUILD.json").write_text(json.dumps({"experimental": args.exp, "tracks": tracks}, indent=2))
    manifest = {name: meta for name, (_, meta) in sorted(PACKAGES.items())}
    (APP / "SYSTEM-PACKAGES.json").write_text(json.dumps(manifest, indent=2))
    # Include Rust dependency licensing/source metadata and available license texts.
    metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1", cwd=ROOT))
    (APP / "RUST-PACKAGES.json").write_text(json.dumps([
        {key: p.get(key) for key in ("name", "version", "license", "repository", "source")}
        for p in metadata["packages"]], indent=2))
    for p in metadata["packages"]:
        if not p.get("source"):
            continue
        for source in Path(p["manifest_path"]).parent.iterdir():
            if source.is_file() and source.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")):
                dest = licenses / "rust" / f'{p["name"]}-{p["version"]}'
                dest.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, dest / source.name)
    (APP / "f1-racecraft.desktop").write_text("[Desktop Entry]\nType=Application\nName=F1 Racecraft\nExec=AppRun\nIcon=f1-racecraft\nCategories=Game;\nTerminal=false\n")
    shutil.copy2(ROOT / "tools/package/f1-racecraft.svg", APP / "f1-racecraft.svg")
    runtime = fetch(RUNTIME_URL, CACHE / "uruntime-v0.8.1-x86_64")
    runtime_license = fetch("https://raw.githubusercontent.com/VHSgunzo/uruntime/v0.8.1/LICENSE", CACHE / "uruntime-LICENSE")
    shutil.copy2(runtime_license, licenses / "uruntime-MIT.txt")
    if hashlib.file_digest(runtime.open("rb"), "sha256").hexdigest() != RUNTIME_SHA:
        raise RuntimeError("Runtime checksum mismatch")
    squash = WORK / "payload.squashfs"
    subprocess.run(["mksquashfs", str(APP), str(squash), "-noappend", "-comp", "zstd", "-Xcompression-level", "15", "-processors", "3", "-all-root"], check=True)
    suffix = "-exp" if args.exp else ""
    output = DIST / f"F1-Racecraft-0.1.0{suffix}-linux-x86_64.AppImage"
    # Automatic no-FUSE fallback, even when this game grows beyond 350 MiB.
    data = runtime.read_bytes().replace(b"URUNTIME_EXTRACT=3", b"URUNTIME_EXTRACT=2")
    with output.open("wb") as target:
        target.write(data)
        with squash.open("rb") as source:
            shutil.copyfileobj(source, target)
    output.chmod(0o755)
    checksum = hashlib.file_digest(output.open("rb"), "sha256").hexdigest()
    output.with_suffix(output.suffix + ".sha256").write_text(f"{checksum}  {output.name}\n")
    print(f"Built {output} ({output.stat().st_size / 1024**2:.1f} MiB)", flush=True)


if __name__ == "__main__":
    main()
