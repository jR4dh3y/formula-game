"""Generate the drift park: a flat asphalt pad with a technical drift course.

    python3 tools/track/generate_drift.py <out dir>

Writes track.json, collision.bin and visual.glb in the converted-track format
(see convert_glb.py). The course is a closed spline of hairpins, a chicane and
long sweepers painted onto a wide asphalt pad, so a car sliding wide stays on
tarmac; grass and a tyre barrier surround the pad. Everything is generated:
no third-party assets.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
from collision import SURFACE_ASPHALT, SURFACE_GRASS, write_collision  # noqa: E402
from glb import GlbMaterial, GlbMesh, write_glb  # noqa: E402

SPACING = 2.0
HALF_WIDTH = 7.0
# Course control points (x, z) in metres, clockwise. Long entry straight,
# a fast sweeper, a hairpin, an S, a tight chicane and a long final bend.
CONTROL = [
    (0, -60), (0, 20), (6, 70), (30, 100), (70, 105), (100, 85),
    (105, 55), (85, 35), (60, 40), (45, 20), (60, 0), (95, -5),
    (120, -25), (120, -60), (95, -85), (55, -80), (40, -100),
    (20, -105), (5, -90),
]
CORNERS = [
    ("Sweeper", 70), ("Hairpin", 230), ("Esses", 330),
    ("Chicane", 420), ("Long Left", 520), ("Last Kink", 640),
]
PAD_MARGIN = 35.0  # asphalt beyond the course in every direction
GRASS = 25.0  # grass between the pad and the barrier


def catmull_rom(points: np.ndarray, samples: int) -> np.ndarray:
    count = len(points)
    out = []
    for i in range(count):
        p0, p1, p2, p3 = (points[(i + k - 1) % count] for k in range(4))
        for t in np.linspace(0, 1, samples, endpoint=False):
            t2, t3 = t * t, t * t * t
            out.append(
                0.5
                * (
                    2 * p1
                    + (-p0 + p2) * t
                    + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2
                    + (-p0 + 3 * p1 - 3 * p2 + p3) * t3
                )
            )
    return np.array(out)


def resample(points: np.ndarray, spacing: float) -> np.ndarray:
    closed = np.vstack([points, points[:1]])
    lengths = np.linalg.norm(np.diff(closed, axis=0), axis=1)
    distance = np.concatenate([[0], np.cumsum(lengths)])
    total = distance[-1]
    count = int(round(total / spacing))
    targets = np.linspace(0, total, count, endpoint=False)
    x = np.interp(targets, distance, closed[:, 0])
    z = np.interp(targets, distance, closed[:, 1])
    return np.stack([x, z], axis=1)


def noise_texture(size, base, spread, seed, speckle=0.0) -> Image.Image:
    rng = np.random.default_rng(seed)
    fine = rng.normal(0, 1, (size, size))
    coarse = np.kron(rng.normal(0, 1, (size // 16, size // 16)), np.ones((16, 16)))
    value = fine * 0.6 + coarse * 0.4
    pixels = np.clip(
        np.array(base)[None, None, :] + value[..., None] * np.array(spread)[None, None, :], 0, 255
    )
    if speckle:
        dots = rng.random((size, size)) < speckle
        pixels[dots] = np.clip(pixels[dots] + 45, 0, 255)
    return Image.fromarray(pixels.astype(np.uint8), "RGB")


def stripes(size, colours) -> Image.Image:
    band = size // len(colours)
    pixels = np.zeros((size, size, 3), np.uint8)
    for i, colour in enumerate(colours):
        pixels[:, i * band : (i + 1) * band] = colour
    return Image.fromarray(pixels, "RGB")


def quad_grid(x0, x1, z0, z1, y, step, uv_scale):
    xs = np.arange(x0, x1 + 1e-6, step)
    zs = np.arange(z0, z1 + 1e-6, step)
    gx, gz = np.meshgrid(xs, zs)
    positions = np.stack([gx, np.full_like(gx, y), gz], axis=-1).reshape(-1, 3)
    uvs = np.stack([gx, gz], axis=-1).reshape(-1, 2) / uv_scale
    nx, nz = len(xs), len(zs)
    indices = []
    for j in range(nz - 1):
        for i in range(nx - 1):
            a = j * nx + i
            indices += [a, a + nx, a + 1, a + 1, a + nx, a + nx + 1]
    return positions, uvs, np.array(indices, np.uint32)


def ring(outer_lo, outer_hi, inner_lo, inner_hi, y):
    """Four quads between two axis-aligned rectangles (x, z)."""
    (ox0, oz0), (ox1, oz1) = outer_lo, outer_hi
    (ix0, iz0), (ix1, iz1) = inner_lo, inner_hi
    quads = [
        ((ox0, oz0), (ox1, oz0), (ox1, iz0), (ox0, iz0)),
        ((ox0, iz1), (ox1, iz1), (ox1, oz1), (ox0, oz1)),
        ((ox0, iz0), (ix0, iz0), (ix0, iz1), (ox0, iz1)),
        ((ix1, iz0), (ox1, iz0), (ox1, iz1), (ix1, iz1)),
    ]
    positions, indices = [], []
    for quad in quads:
        base = len(positions)
        positions += [(x, y, z) for x, z in quad]
        indices += [base, base + 3, base + 1, base + 1, base + 3, base + 2]
    positions = np.array(positions, float)
    return positions, positions[:, [0, 2]] / 8.0, np.array(indices, np.uint32)


def strip(points, left_offset, right_offset, y, closed=True, v_scale=4.0):
    """A ribbon along `points` between two lateral offsets (left positive)."""
    tangents = np.roll(points, -1, axis=0) - np.roll(points, 1, axis=0)
    tangents /= np.linalg.norm(tangents, axis=1, keepdims=True)
    left = np.stack([-tangents[:, 1], tangents[:, 0]], axis=1)
    a = points + left * left_offset
    b = points + left * right_offset
    count = len(points)
    positions = np.zeros((count * 2, 3))
    positions[0::2, [0, 2]] = a
    positions[1::2, [0, 2]] = b
    positions[:, 1] = y
    along = np.arange(count) * SPACING / v_scale
    uvs = np.zeros((count * 2, 2))
    uvs[0::2] = np.stack([np.zeros(count), along], axis=1)
    uvs[1::2] = np.stack([np.ones(count), along], axis=1)
    indices = []
    segments = count if closed else count - 1
    for i in range(segments):
        j = (i + 1) % count
        indices += [2 * i, 2 * j, 2 * i + 1, 2 * i + 1, 2 * j, 2 * j + 1]
    return positions, uvs, np.array(indices, np.uint32)


def box(centre, size, yaw=0.0):
    cx, cy, cz = centre
    sx, sy, sz = (s / 2 for s in size)
    c, s = math.cos(yaw), math.sin(yaw)
    corners = []
    for dx, dy, dz in [
        (-sx, -sy, -sz), (sx, -sy, -sz), (sx, sy, -sz), (-sx, sy, -sz),
        (-sx, -sy, sz), (sx, -sy, sz), (sx, sy, sz), (-sx, sy, sz),
    ]:
        corners.append((cx + dx * c - dz * s, cy + dy, cz + dx * s + dz * c))
    faces = [(0, 1, 2, 3), (5, 4, 7, 6), (4, 0, 3, 7), (1, 5, 6, 2), (3, 2, 6, 7), (4, 5, 1, 0)]
    positions, uvs, indices = [], [], []
    for face in faces:
        base = len(positions)
        for k, index in enumerate(face):
            positions.append(corners[index])
            uvs.append([(0, 0), (1, 0), (1, 1), (0, 1)][k])
        indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    return np.array(positions), np.array(uvs, float), np.array(indices, np.uint32)


def normals_for(positions, indices):
    normals = np.zeros_like(positions)
    tri = indices.reshape(-1, 3)
    face = np.cross(
        positions[tri[:, 1]] - positions[tri[:, 0]], positions[tri[:, 2]] - positions[tri[:, 0]]
    )
    for k in range(3):
        np.add.at(normals, tri[:, k], face)
    length = np.linalg.norm(normals, axis=1, keepdims=True)
    normals /= np.where(length > 0, length, 1)
    # Flat ground faces up whichever way it was wound.
    flat = np.abs(normals[:, 1]) > 0.99
    normals[flat] = [0, 1, 0]
    return normals


class Builder:
    def __init__(self):
        self.parts: dict[int, list] = {}

    def add(self, material, positions, uvs, indices):
        self.parts.setdefault(material, []).append((positions, uvs, indices))

    def meshes(self):
        out = []
        for material, parts in self.parts.items():
            positions, uvs, indices, offset = [], [], [], 0
            for p, u, i in parts:
                positions.append(p)
                uvs.append(u)
                indices.append(i + offset)
                offset += len(p)
            positions = np.vstack(positions)
            indices = np.concatenate(indices)
            out.append(
                GlbMesh(
                    name=f"part-{material}",
                    material=material,
                    positions=positions.astype(np.float32),
                    normals=normals_for(positions, indices).astype(np.float32),
                    uvs=np.vstack(uvs).astype(np.float32),
                    indices=indices.astype(np.uint32),
                )
            )
        return out


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    spline = catmull_rom(np.array(CONTROL, float), 40)
    course = resample(spline, SPACING)
    length = len(course) * SPACING

    lo = course.min(axis=0) - PAD_MARGIN
    hi = course.max(axis=0) + PAD_MARGIN
    grass_lo, grass_hi = lo - GRASS, hi + GRASS

    materials = [
        GlbMaterial("pad_asphalt", image=noise_texture(512, (62, 63, 66), (10, 10, 10), 1, 0.01),
                    roughness=0.9),
        GlbMaterial("course_asphalt", image=noise_texture(512, (44, 45, 48), (8, 8, 8), 2, 0.02),
                    roughness=0.85),
        GlbMaterial("paint_white", base_color=(0.92, 0.92, 0.9, 1.0), roughness=0.6),
        GlbMaterial("grass", image=noise_texture(512, (58, 96, 44), (12, 16, 10), 3), roughness=1.0),
        GlbMaterial("tyre_barrier", image=stripes(64, [(20, 20, 22), (200, 30, 30), (20, 20, 22),
                                                         (230, 230, 230)]), roughness=0.95),
        GlbMaterial("cone", image=stripes(64, [(240, 90, 10), (240, 240, 240), (240, 90, 10)]),
                    roughness=0.7),
        GlbMaterial("steel", base_color=(0.55, 0.57, 0.6, 1.0), metallic=0.8, roughness=0.4),
        GlbMaterial("kerb", image=stripes(64, [(210, 30, 30), (235, 235, 235)]), roughness=0.7),
    ]
    PAD, COURSE, PAINT, GRASS_M, BARRIER, CONE, STEEL, KERB = range(len(materials))
    builder = Builder()

    builder.add(PAD, *quad_grid(lo[0], hi[0], lo[1], hi[1], 0.0, 10.0, 8.0))
    builder.add(GRASS_M, *ring(tuple(grass_lo), tuple(grass_hi), tuple(lo), tuple(hi), -0.02))
    builder.add(COURSE, *strip(course, HALF_WIDTH, -HALF_WIDTH, 0.01, v_scale=8.0))
    for edge in (HALF_WIDTH, -HALF_WIDTH):
        builder.add(PAINT, *strip(course, edge + 0.15, edge - 0.15, 0.02))
    # Start/finish line across the course.
    start, after = course[0], course[1]
    tangent = (after - start) / np.linalg.norm(after - start)
    builder.add(PAINT, *box((start[0], 0.02, start[1]), (HALF_WIDTH * 2, 0.01, 0.6),
                            math.atan2(tangent[0], tangent[1])))

    # Kerbs and cones at the apex of each named corner, on the inside.
    curvature = []
    for i in range(len(course)):
        a, b, c = course[i - 4], course[i], course[(i + 4) % len(course)]
        u, v = b - a, c - b
        curvature.append((u[0] * v[1] - u[1] * v[0]) / (np.linalg.norm(u) * np.linalg.norm(v) * 8))
    curvature = np.array(curvature)
    corners = []
    for name, approx in CORNERS:
        window = range(int(approx / SPACING) - 25, int(approx / SPACING) + 25)
        index = max(window, key=lambda i: abs(curvature[i % len(course)])) % len(course)
        corners.append({"name": name, "distance": round(index * SPACING, 1)})
        inside = 1.0 if curvature[index] > 0 else -1.0
        segment = course[max(0, index - 6) : index + 7]
        if len(segment) > 3:
            builder.add(KERB, *strip(segment, inside * (HALF_WIDTH + 1.2), inside * HALF_WIDTH,
                                     0.03, closed=False, v_scale=2.0))
        tangent = course[(index + 1) % len(course)] - course[index - 1]
        tangent /= np.linalg.norm(tangent)
        normal = np.array([-tangent[1], tangent[0]]) * inside
        for k in (-6, 0, 6):
            p = course[(index + k) % len(course)] + normal * (HALF_WIDTH + 2.5)
            builder.add(CONE, *box((p[0], 0.35, p[1]), (0.35, 0.7, 0.35)))
    corners.sort(key=lambda corner: corner["distance"])

    # Tyre barrier on the pad edge, with a gap-free wall for collision.
    walls = []
    wall_lo, wall_hi = grass_lo + 3.0, grass_hi - 3.0
    edge = [(wall_lo[0], wall_lo[1]), (wall_hi[0], wall_lo[1]),
            (wall_hi[0], wall_hi[1]), (wall_lo[0], wall_hi[1])]
    for i in range(4):
        (x0, z0), (x1, z1) = edge[i], edge[(i + 1) % 4]
        walls.append((x0, z0, x1, z1))
        span = math.hypot(x1 - x0, z1 - z0)
        yaw = math.atan2(x1 - x0, z1 - z0)
        for k in np.arange(0.6, span, 1.2):
            t = k / span
            builder.add(BARRIER, *box((x0 + (x1 - x0) * t, 0.5, z0 + (z1 - z0) * t),
                                      (1.0, 1.0, 1.2), yaw))
    # Light masts around the pad.
    for x in np.linspace(lo[0] + 10, hi[0] - 10, 5):
        for z in (lo[1] - 6, hi[1] + 6):
            builder.add(STEEL, *box((x, 7.5, z), (0.35, 15.0, 0.35)))
            builder.add(STEEL, *box((x, 15.2, z), (2.4, 0.4, 0.8)))

    # Collision: the pad (asphalt) and grass ring as a coarse grid.
    triangles, surfaces = [], []
    for (x0, x1, z0, z1, surface) in [
        (lo[0], hi[0], lo[1], hi[1], SURFACE_ASPHALT),
        (grass_lo[0], hi[0], grass_lo[1], lo[1], SURFACE_GRASS),
        (lo[0], grass_hi[0], hi[1], grass_hi[1], SURFACE_GRASS),
        (grass_lo[0], lo[0], lo[1], grass_hi[1], SURFACE_GRASS),
        (hi[0], grass_hi[0], grass_lo[1], hi[1], SURFACE_GRASS),
    ]:
        xs = np.linspace(x0, x1, max(2, int((x1 - x0) / 10) + 1))
        zs = np.linspace(z0, z1, max(2, int((z1 - z0) / 10) + 1))
        for i in range(len(xs) - 1):
            for j in range(len(zs) - 1):
                a = (xs[i], 0.0, zs[j]); b = (xs[i + 1], 0.0, zs[j])
                c = (xs[i + 1], 0.0, zs[j + 1]); d = (xs[i], 0.0, zs[j + 1])
                triangles += [a + b + c, a + c + d]
                surfaces += [surface, surface]

    write_collision(str(out / "collision.bin"), np.array(triangles), np.array(surfaces), np.array(walls))
    write_glb(str(out / "visual.glb"), materials, builder.meshes(), 512)
    metadata = {
        "name": "Racecraft Drift Park",
        "source": "Generated by tools/track/generate_drift.py",
        "length": round(length, 2),
        "centerline": [[round(x, 3), 0.0, round(z, 3)] for x, z in course],
        "half_width": [HALF_WIDTH] * len(course),
        "corners": corners,
        "sun_direction": [-0.35, -0.8, -0.48],
    }
    (out / "track.json").write_text(json.dumps(metadata, separators=(",", ":")) + "\n")
    print(json.dumps({"length": metadata["length"], "stations": len(course), "corners": corners}))


if __name__ == "__main__":
    main()
