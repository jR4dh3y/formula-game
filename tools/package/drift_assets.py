"""Include drift mode only when both converted asset sets are complete."""
from pathlib import Path
import shutil


FILES = {
    "cars/thar": ("car.json", "car.glb", "car_lod1.glb"),
    "tracks/driftpark": ("track.json", "collision.bin", "visual.glb"),
}


def copy_drift_assets(source: Path, destination: Path) -> bool:
    if not any((source / directory).exists() for directory in FILES):
        return False
    for directory, names in FILES.items():
        for name in names:
            path = source / directory / name
            if not path.is_file():
                raise RuntimeError(f"Missing drift asset: {path}")
    for directory in FILES:
        shutil.copytree(source / directory, destination / directory)
    return True
