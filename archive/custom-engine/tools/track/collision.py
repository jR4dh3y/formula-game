"""Binary collision format shared with `racecraft-sim` (sim/src/track/collision.rs).

Layout (little endian):
  magic       b"F1CL"
  version     u32 = 1
  triangles   u32
  walls       u32
  positions   f32[triangles * 9]   drivable triangle corners
  surfaces    u8[triangles]        SURFACE_* ids
  padding     to 4 bytes
  segments    f32[walls * 4]       x0, z0, x1, z1 in the ground plane
"""

import struct

import numpy as np

SURFACE_ASPHALT = 0
SURFACE_KERB = 1
SURFACE_GRASS = 2
SURFACE_GRAVEL = 3
SURFACE_CONCRETE = 4


def write_collision(path: str, triangles: np.ndarray, surfaces: np.ndarray, walls: np.ndarray):
    triangles = np.ascontiguousarray(triangles, dtype="<f4").reshape(-1, 9)
    surfaces = np.ascontiguousarray(surfaces, dtype=np.uint8)
    walls = np.ascontiguousarray(walls, dtype="<f4").reshape(-1, 4)
    assert len(triangles) == len(surfaces)
    with open(path, "wb") as handle:
        handle.write(b"F1CL")
        handle.write(struct.pack("<III", 1, len(triangles), len(walls)))
        handle.write(triangles.tobytes())
        handle.write(surfaces.tobytes())
        handle.write(b"\0" * ((4 - len(surfaces) % 4) % 4))
        handle.write(walls.tobytes())
