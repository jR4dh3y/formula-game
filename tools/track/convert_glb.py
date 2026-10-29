"""Convert a static visual GLB using an explicit material/layout configuration.

Usage: python3 tools/track/convert_glb.py CONFIG SOURCE.glb OUTPUT_DIRECTORY
Dependencies: tools/track/requirements.txt. See tools/track/README.md.
"""

from __future__ import annotations

import argparse
import fnmatch
import itertools
import json
from pathlib import Path

import fast_simplification
import numpy as np
from centerline import extract
from collision import write_collision
from glb import GlbMaterial, GlbMesh, write_glb
from PIL import Image, ImageDraw
from read_glb import Glb

SURFACES = ("asphalt", "kerb", "grass", "gravel", "concrete")


def matches(name, patterns):
    return any(fnmatch.fnmatchcase(name, p) for p in patterns)


def surface(name, config):
    return next(
        (i for i, key in enumerate(SURFACES) if matches(name, config[key])), None
    )


def walls_from(triangles, points):
    """Intersect tall vertical faces at bumper height, excluding overhead rails."""
    from scipy.spatial import cKDTree

    cross = np.cross(
        triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0]
    )
    norm = np.linalg.norm(cross, axis=1)
    tall = np.ptp(triangles[:, :, 1], axis=1) >= 0.4
    vertical = np.abs(cross[:, 1]) < 0.5 * norm
    triangles = triangles[tall & vertical & (norm > 1e-8)]
    tree = cKDTree(points[:, [0, 2]])
    distance, station = tree.query(triangles[:, :, [0, 2]].mean(axis=1))
    heights = np.where(
        distance <= 50, points[station, 1] + 0.2, triangles[:, :, 1].min(axis=1) + 0.2
    )
    segments = []
    for tri, height in zip(triangles, heights):
        hits = []
        for a, b in zip(tri, np.roll(tri, -1, axis=0)):
            if abs(b[1] - a[1]) < 1e-7 or not min(a[1], b[1]) <= height <= max(
                a[1], b[1]
            ):
                continue
            fraction = (height - a[1]) / (b[1] - a[1])
            hits.append((a + (b - a) * fraction)[[0, 2]])
        if len(hits) < 2:
            continue
        a, b = max(
            itertools.combinations(hits, 2),
            key=lambda pair: np.linalg.norm(pair[1] - pair[0]),
        )
        if np.linalg.norm(b - a) < 0.05:
            continue
        if tuple(a) > tuple(b):
            a, b = b, a
        segments.append(np.r_[a, b])
    return np.unique(np.round(segments, 2), axis=0).astype("f4").reshape(-1, 4)


def simplify(p, n, u, f, target):
    if target >= len(f):
        return p, n, u, f
    vertices, inverse = np.unique(f, return_inverse=True)
    p, n, u = p[vertices], n[vertices], u[vertices]
    f = inverse.reshape(-1, 3)
    # Collapse only within one primitive/material. Separate UV seam vertices
    # remain separate; replay supplies the authoritative original->new map.
    _, _, collapses = fast_simplification.simplify(
        p, f.astype("i4"), target_count=target, return_collapses=True
    )
    positions, faces, mapping = fast_simplification.replay_simplification(
        p, f.astype("i4"), collapses
    )
    valid = mapping >= 0
    counts = np.bincount(mapping[valid], minlength=len(positions)).clip(1)
    attrs = np.zeros((len(positions), 5), dtype="f4")
    np.add.at(attrs, mapping[valid], np.column_stack([n, u])[valid])
    attrs /= counts[:, None]
    normals = attrs[:, :3]
    normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-12)
    return positions.astype("f4"), normals, attrs[:, 3:], faces


def debug_map(path, triangles, surfaces, walls, points):
    lo = triangles[:, :, [0, 2]].min(axis=(0, 1)) - 10
    hi = triangles[:, :, [0, 2]].max(axis=(0, 1)) + 10
    scale = 1700 / max(hi - lo)
    size = np.ceil((hi - lo) * scale).astype(int)
    image = Image.new("RGB", tuple(size), "#20262c")
    draw = ImageDraw.Draw(image)
    colors = ["#666971", "#cb5849", "#416c3c", "#bca06a", "#949697"]

    def pixel(p):
        return tuple((np.asarray(p) - lo) * scale)

    for sid in [2, 3, 4, 0, 1]:
        for tri in triangles[surfaces == sid][:, :, [0, 2]]:
            draw.polygon([pixel(p) for p in tri], fill=colors[sid])
    for wall in walls:
        draw.line([pixel(wall[:2]), pixel(wall[2:])], fill="#ffdc80", width=1)
    draw.line(
        [pixel(p[[0, 2]]) for p in points] + [pixel(points[0, [0, 2]])],
        fill="#00ffff",
        width=2,
    )
    for i, p in enumerate(points[::100]):
        draw.text(pixel(p[[0, 2]]), str(i * 300), fill="white")
    p = pixel(points[0, [0, 2]])
    draw.ellipse((p[0] - 6, p[1] - 6, p[0] + 6, p[1] + 6), fill="#ff0000")
    draw.line(
        [p, pixel(points[min(5, len(points) - 1), [0, 2]])], fill="#ff0000", width=4
    )
    image.save(path)


def convert(config_path, source, destination, centerline_only=False):
    config = json.loads(Path(config_path).read_text())
    out = Path(destination)
    out.mkdir(parents=True, exist_ok=True)
    model = Glb(source)
    source_materials = model.json["materials"]
    materials = []
    for m in source_materials:
        name = m.get("name", "unnamed")
        sid = surface(name, config)
        pbr = m.get("pbrMetallicRoughness", {})
        image = (
            None if centerline_only else model.image(m, config.get("max_texture", 1024))
        )
        alpha = m.get("alphaMode") == "MASK"
        # Assetto billboard atlases sometimes arrive as BLEND. Preserve their
        # transparent texels as cutouts instead of turning trees/fences opaque.
        if image is not None and image.mode == "RGBA":
            a = np.asarray(image.getchannel("A"))
            alpha = alpha or bool((a < 128).mean() > 0.01)
        materials.append(
            GlbMaterial(
                name=("ground:" if sid is not None else "") + name,
                base_color=tuple(pbr.get("baseColorFactor", [1, 1, 1, 1])),
                image=image,
                alpha_mask=alpha,
                roughness=pbr.get("roughnessFactor", 0.8),
                metallic=pbr.get("metallicFactor", 0),
                emissive=tuple(m.get("emissiveFactor", [0, 0, 0])),
            )
        )
    total = 0
    protected = 0
    for m, p, n, u, f in model.primitives(config.get("scale", 1)):
        total += len(f)
        if matches(
            source_materials[m].get("name", ""), config["road"] + config["kerb"]
        ):
            protected += len(f)
    budget = config.get("max_triangles", 950000)
    ratio = min(1, max(0, (budget - protected) / max(1, total - protected)))
    print(
        f"{config['name']}: {total} source triangles; scenery ratio {ratio:.3f}",
        flush=True,
    )
    buckets = {}
    collision = []
    ids = []
    wall_parts = []
    road = []
    route = []
    edges = []
    for m, p, n, u, f in model.primitives(config.get("scale", 1)):
        name = source_materials[m].get("name", "")
        if matches(name, config.get("omit", [])):
            continue
        sid = surface(name, config)
        triangles = p[f]
        cross = np.cross(
            triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0]
        )
        flat = np.abs(cross[:, 1]) > np.linalg.norm(cross, axis=1) * 0.6
        if sid is not None and flat.any():
            collision.append(triangles[flat])
            ids.append(np.full(flat.sum(), sid, dtype="u1"))
        if matches(name, config["wall"]):
            wall_parts.append(triangles)
        if matches(name, config["road"]):
            road.append(triangles[flat])
        if matches(name, config.get("route", [])):
            route.append(triangles[flat])
        if matches(name, config.get("edges", [])):
            edges.append(triangles[flat])
        if centerline_only:
            continue
        keep = matches(name, config["road"] + config["kerb"])
        if not keep and ratio < 1 and len(f) > 20:
            print(f"  decimate {name}: {len(f)} triangles", flush=True)
            p, n, u, f = simplify(p, n, u, f, max(4, int(len(f) * ratio)))
        centers = p[f].mean(axis=1)
        tiles = np.floor(centers[:, [0, 2]] / 512).astype(int)
        for tile in np.unique(tiles, axis=0):
            faces = f[(tiles == tile).all(axis=1)]
            vertices, inverse = np.unique(faces, return_inverse=True)
            buckets.setdefault((m, *tile), []).append(
                (p[vertices], n[vertices], u[vertices], inverse.reshape(-1, 3))
            )
    road = np.concatenate(road)
    points, widths = extract(
        road,
        config,
        out / "road-debug.png",
        np.concatenate(route) if route else None,
        np.concatenate(edges) if edges else None,
    )
    steps = np.linalg.norm(np.roll(points, -1, axis=0) - points, axis=1)
    length = float(steps.sum())
    if abs(length / config["length"] - 1) > 0.02:
        raise ValueError("final 3D lap length outside tolerance")
    corners = [
        {"name": name, "distance": round(float(distance), 1)}
        for name, distance in config.get("corners", [])
    ]
    distances = np.r_[0, np.cumsum(steps[:-1])]
    for name, position in config.get("corner_positions", []):
        index = np.argmin(np.linalg.norm(points[:, [0, 2]] - position, axis=1))
        corners.append({"name": name, "distance": round(float(distances[index]), 1)})
    corners.sort(key=lambda corner: corner["distance"])
    metadata = {
        "name": config["name"],
        "source": config["source"],
        "length": round(length, 2),
        "centerline": np.round(points, 3).tolist(),
        "half_width": np.round(widths, 3).tolist(),
        "corners": corners,
        "sun_direction": config["sun_direction"],
    }
    triangles = np.concatenate(collision)
    surfaces = np.concatenate(ids)
    if config.get("infill"):
        # The 24H visual mesh omits pieces of road hidden beneath bridges.
        # Reconstruct those collision patches from the painted road edges.
        # Existing source geometry remains the authority everywhere it exists.
        from centerline import road_grid

        coverage, origin = road_grid(road, close=False)
        tangent = np.roll(points[:, [0, 2]], -1, axis=0) - np.roll(
            points[:, [0, 2]], 1, axis=0
        )
        tangent /= np.maximum(np.linalg.norm(tangent, axis=1, keepdims=True), 1e-9)
        left = np.column_stack([-tangent[:, 1], tangent[:, 0]])
        side = left * widths[:, None]
        a = points.copy()
        b = points.copy()
        a[:, [0, 2]] += side
        b[:, [0, 2]] -= side
        patches = []
        for i in range(len(points)):
            j = (i + 1) % len(points)
            midpoint = (points[i, [0, 2]] + points[j, [0, 2]]) / 2
            q = np.rint((midpoint - origin) / 0.5).astype(int)
            if not coverage[q[1], q[0]]:
                patches.extend([[a[i], b[i], a[j]], [b[i], b[j], a[j]]])
        if patches:
            triangles = np.concatenate([triangles, np.array(patches, dtype="f4")])
            surfaces = np.r_[surfaces, np.zeros(len(patches), dtype="u1")]
        print(f"infilled {len(patches)} hidden road triangles", flush=True)
    walls = (
        walls_from(np.concatenate(wall_parts), points)
        if wall_parts
        else np.empty((0, 4))
    )
    meshes = []
    for (m, x, z), parts in sorted(buckets.items()):
        offsets = np.cumsum([0] + [len(part[0]) for part in parts[:-1]])
        meshes.append(
            GlbMesh(
                f"{materials[m].name}@{x}_{z}",
                m,
                np.concatenate([p[0] for p in parts]),
                np.concatenate([p[1] for p in parts]),
                np.concatenate([p[2] for p in parts]),
                np.concatenate(
                    [part[3] + offset for part, offset in zip(parts, offsets)]
                ).reshape(-1),
            )
        )
    rendered = sum(len(m.indices) // 3 for m in meshes)
    if rendered > 1_000_000:
        raise ValueError(f"visual triangle budget exceeded: {rendered}")
    if not centerline_only:
        write_glb(
            str(out / "visual.glb"), materials, meshes, config.get("max_texture", 1024)
        )
    write_collision(str(out / "collision.bin"), triangles, surfaces, walls)
    (out / "track.json").write_text(json.dumps(metadata, separators=(",", ":")) + "\n")
    debug_map(out / "debug.png", triangles, surfaces, walls, points)
    report = {
        "triangles": rendered,
        "textures": sum(m.image is not None for m in materials),
        "texture_bytes": sum(
            m.image.width * m.image.height * 4 for m in materials if m.image is not None
        ),
        "collision_triangles": len(triangles),
        "walls": len(walls),
        "length": round(length, 2),
        "reference_length": config["length"],
        "minimum_half_width": round(float(widths.min()), 2),
    }
    if centerline_only:
        previous = json.loads((out / "conversion.json").read_text())
        for key in ["triangles", "textures", "texture_bytes"]:
            report[key] = previous[key]
    (out / "conversion.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config")
    parser.add_argument("source")
    parser.add_argument("destination")
    parser.add_argument(
        "--centerline-only",
        action="store_true",
        help="reuse visual.glb while refining road metadata/collision",
    )
    args = parser.parse_args()
    convert(args.config, args.source, args.destination, args.centerline_only)
