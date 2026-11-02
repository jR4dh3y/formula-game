"""Rebuild only the wing arrays of existing livery maps.

Usage: python3 -I tools/livery/update_wings.py [--work DIR]

The body solve in build_2026.sh takes minutes and does not read the wing
arrays, which build_map_v3.py copies verbatim from map2026.npz. This recomputes
front_wing_* and rear_wing_* with wing_map.py and writes them into both
map2026.npz and map_v3.npz. Every other array is copied byte for byte, and the
region names only gain entries at the end, so existing indices are unchanged.
A full build_2026.sh run produces the same wing arrays.
"""
import argparse
import os
from pathlib import Path
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import wing_map

REPO = Path(__file__).resolve().parents[2]
PARTS = (("front_wing", wing_map.front_wing), ("rear_wing", wing_map.rear_wing))
NEW_NAMES = ("fw_ep_in_right", "fw_ep_in_left", "fw_flap_top", "fw_flap_mid", "fw_main",
             "rw_main_top", "rw_flap_lo", "rw_flap_hi", "rw_flap_back", "rw_main_under")


def update(path, wings, names):
    old = np.load(path)
    old_names = [str(n) for n in old["names"]]
    if names[:len(old_names)] != old_names:
        raise SystemExit(f"{path}: region names are not a prefix of the new list")
    arrays = {k: old[k] for k in old.files}
    arrays["names"] = np.array(names)
    for part, packed in wings.items():
        for key, value in packed.items():
            arrays[f"{part}_{key}"] = value
    tmp = path.with_name(path.stem + ".tmp.npz")
    np.savez_compressed(tmp, **arrays)
    new = np.load(tmp)
    for k in old.files:
        if k == "names" or k.startswith(("front_wing_", "rear_wing_")):
            continue
        if not np.array_equal(old[k], new[k], equal_nan=old[k].dtype.kind == "f"):
            raise SystemExit(f"{path}: {k} changed")
    os.replace(tmp, path)
    print(f"{path}: wings replaced, {len(old.files) - 1 - 8} other arrays unchanged")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--work", type=Path, default=REPO / "raw/gen22/livery-mapping")
    args = ap.parse_args()
    L = np.load(args.work / "islands.npy")
    base = [str(n) for n in np.load(args.work / "map2026.npz")["names"]]
    names = base + [n for n in NEW_NAMES if n not in base]
    wings = {}
    for part, build in PARTS:
        d = np.load(args.work / "bake2k" / f"{part}.npz")
        wings[part] = wing_map.pack(build(d, L), d["mask"], names, L)
        print(part, "mapped", int((wings[part]["region"] >= 0).sum()), flush=True)
    for name in ("map2026.npz", "map_v3.npz"):
        update(args.work / name, wings, names)


if __name__ == "__main__":
    main()
