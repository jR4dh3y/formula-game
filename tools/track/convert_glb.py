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


def convert_material(model, material, config, centerline_only=False):
    """Repair known exporter alpha mistakes explicitly, never by texture guessing.

    Legacy racing textures also store specular/roughness data in alpha. A low
    alpha value alone therefore cannot distinguish a fence from opaque asphalt.
    """
    name = material.get("name", "unnamed")
    sid = surface(name, config)
    pbr = dict(material.get("pbrMetallicRoughness", {}))
    # Explicit audited repairs for legacy shaders exported as metallic surfaces.
    for pattern, properties in config.get("material_pbr", {}).items():
        if matches(name, [pattern]):
            if set(properties) - {"metallicFactor", "roughnessFactor", "baseColorFactor"}:
                raise ValueError(f"unsupported PBR override for {name}")
            pbr.update(properties)
    # Some legacy exporters placed diffuse artwork in a specular slot. Select
    # only explicitly audited source images; never reinterpret other tracks.
    texture_rules = [
        image_name
        for pattern, image_name in config.get("material_base_color_texture", {}).items()
        if matches(name, [pattern])
    ]
    if len(texture_rules) > 1:
        raise ValueError(f"conflicting base color image rules for {name}")
    image_material = material
    if texture_rules:
        image_name = texture_rules[0]
        sources = [
            i for i, entry in enumerate(model.json.get("images", []))
            if entry.get("name") == image_name
        ]
        if len(sources) != 1:
            raise ValueError(f"expected one source image {image_name!r} for {name}")
        textures = [
            i for i, entry in enumerate(model.json.get("textures", []))
            if entry.get("source") == sources[0]
        ]
        if not textures:
            raise ValueError(f"source image {image_name!r} has no texture for {name}")
        image_material = {
            **material,
            "pbrMetallicRoughness": {**pbr, "baseColorTexture": {"index": textures[0]}},
        }
    image = None if centerline_only else model.image(image_material, config.get("max_texture", 1024))
    mode = material.get("alphaMode", "OPAQUE")
    overrides = config.get("material_alpha", {})
    selected = [key for key, patterns in overrides.items() if matches(name, patterns)]
    if len(selected) > 1 or any(key not in ("OPAQUE", "MASK", "BLEND") for key in selected):
        raise ValueError(f"invalid/conflicting alpha rules for {name}: {selected}")
    if selected:
        mode = selected[0]
    if image is not None and image.mode == "RGBA" and mode == "OPAQUE":
        image = image.convert("RGB")
    return GlbMaterial(
        name=("ground:" if sid is not None else "")
        + ("decal:" if matches(name, config.get("decals", [])) else "")
        + name,
        base_color=tuple(pbr.get("baseColorFactor", [1, 1, 1, 1])),
        image=image,
        alpha_mask=mode == "MASK",
        alpha_blend=mode == "BLEND",
        alpha_cutoff=material.get("alphaCutoff", 0.5),
        roughness=pbr.get("roughnessFactor", 0.8),
        metallic=pbr.get("metallicFactor", 0),
        emissive=tuple(material.get("emissiveFactor", [0, 0, 0])),
    )


def walls_from(triangles, points, min_face_height=0.4, probe_height=0.2):
    """Intersect tall vertical faces at bumper height, excluding overhead rails."""
    from scipy.spatial import cKDTree

    cross = np.cross(
        triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0]
    )
    norm = np.linalg.norm(cross, axis=1)
    tall = np.ptp(triangles[:, :, 1], axis=1) >= min_face_height
    vertical = np.abs(cross[:, 1]) < 0.5 * norm
    triangles = triangles[tall & vertical & (norm > 1e-8)]
    tree = cKDTree(points[:, [0, 2]])
    distance, station = tree.query(triangles[:, :, [0, 2]].mean(axis=1))
    heights = np.where(
        distance <= 50, points[station, 1] + probe_height,
        triangles[:, :, 1].min(axis=1) + probe_height
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


def widths_inside_walls(points, widths, walls):
    """Constrain route widths where asphalt continues behind a physical barrier."""
    from scipy.ndimage import minimum_filter1d

    def cross(a, b):
        return a[..., 0] * b[..., 1] - a[..., 1] * b[..., 0]

    flat = np.asarray(points, dtype=float)[:, [0, 2]]
    tangent = np.roll(flat, -1, axis=0) - np.roll(flat, 1, axis=0)
    tangent /= np.linalg.norm(tangent, axis=1, keepdims=True)
    normals = np.column_stack([-tangent[:, 1], tangent[:, 0]])
    a = walls[:, :2]
    delta = walls[:, 2:] - a
    limits = []
    for point, normal in zip(flat, normals):
        det = cross(normal, delta)
        valid = np.abs(det) > 1e-7
        safe = np.where(valid, det, 1)
        along = cross(a - point, delta) / safe
        fraction = cross(a - point, normal) / safe
        hit = valid & (fraction >= 0) & (fraction <= 1)
        limits.append(np.abs(along[hit]).min() if hit.any() else float("inf"))
    # A car occupies several metres longitudinally, not a single cross section.
    limits = minimum_filter1d(limits, size=5, mode="wrap")
    result = np.minimum(widths, limits)
    if result.min() < 1.5:
        raise ValueError("route too close to a wall; repair its centreline")
    return result


def bridge_road_seams(road, rules):
    """Underpin audited narrow source cracks between matching road boundary edges.

    The source kerb remains above these patches. Only parallel edges with matching
    endpoints and elevation can join; large gaps and vertical steps stay open.
    """
    if not rules:
        return np.empty((0, 3, 3), dtype="f4")
    from scipy.spatial import cKDTree

    vertices, inverse = np.unique(np.round(road.reshape(-1, 3), 4), axis=0, return_inverse=True)
    faces = inverse.reshape(-1, 3)
    edges = np.concatenate([faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]])
    edges = np.sort(edges, axis=1)
    edges, counts = np.unique(edges, axis=0, return_counts=True)
    boundary = vertices[edges[counts == 1]]
    patches = []
    for rule in rules:
        lo, hi = np.asarray(rule["bounds"])
        inside = ((boundary[:, :, [0, 2]] >= lo) & (boundary[:, :, [0, 2]] <= hi)).all(axis=(1, 2))
        selected = boundary[inside]
        if not len(selected):
            continue
        gap = rule["max_gap"]
        for i, j in sorted(cKDTree(selected.mean(axis=1)[:, [0, 2]]).query_pairs(gap)):
            a, b = selected[i], selected[j]
            if np.linalg.norm(a[0] - b[1]) < np.linalg.norm(a[0] - b[0]):
                b = b[::-1]
            delta = a - b
            if np.max(np.linalg.norm(delta[:, [0, 2]], axis=1)) > gap or np.max(np.abs(delta[:, 1])) > 0.03:
                continue
            u, v = a[1] - a[0], b[1] - b[0]
            lengths = np.linalg.norm([u, v], axis=1)
            if lengths.min() < 1 or lengths.max() / lengths.min() > 1.1:
                continue
            if abs(np.dot(u, v)) / np.prod(lengths) < 0.995:
                continue
            cross = np.cross(u, b[0] - a[0])
            if np.linalg.norm(cross) < 0.005 or abs(cross[1]) < np.linalg.norm(cross) * 0.95:
                continue
            patches.extend([[a[0], a[1], b[0]], [a[1], b[1], b[0]]])
    return np.asarray(patches, dtype="f4").reshape(-1, 3, 3)


def remove_placeholders(positions, faces, material, rules):
    """Remove configured disconnected authoring cubes, preserving terrain."""
    matching = [rule for rule in rules if matches(material, [rule["material"]])]
    if not matching:
        return faces
    from scipy.sparse import coo_matrix
    from scipy.sparse.csgraph import connected_components

    vertices, inverse = np.unique(np.round(positions, 5), axis=0, return_inverse=True)
    triangles = inverse[faces]
    a = triangles.reshape(-1)
    b = np.roll(triangles, -1, axis=1).reshape(-1)
    graph = coo_matrix((np.ones(len(a)), (a, b)), shape=(len(vertices), len(vertices)))
    count, labels = connected_components(graph, directed=False)
    omitted = set()
    for index in range(count):
        component = vertices[labels == index]
        if len(component) != 8:
            continue
        dimensions = np.ptp(component, axis=0)
        if any(
            np.allclose(
                dimensions, rule["dimensions"], atol=rule.get("tolerance", 0.05), rtol=0
            )
            for rule in matching
        ):
            omitted.add(index)
    return faces[~np.isin(labels[triangles[:, 0]], list(omitted))]


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
    materials = [
        convert_material(model, m, config, centerline_only) for m in source_materials
    ]
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
        f = remove_placeholders(p, f, name, config.get("omit_boxes", []))
        if not len(f):
            continue
        sid = surface(name, config)
        triangles = p[f]
        cross = np.cross(
            triangles[:, 1] - triangles[:, 0], triangles[:, 2] - triangles[:, 0]
        )
        flat = np.abs(cross[:, 1]) > np.linalg.norm(cross, axis=1) * 0.6
        if sid is not None and flat.any() and not matches(name, config.get("decals", [])):
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
        raise ValueError(f"final 3D lap length {length:.1f} outside tolerance")
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
        walls_from(
            np.concatenate(wall_parts), points,
            config.get("wall_min_face_height", 0.4),
            config.get("wall_probe_height", 0.2),
        )
        if wall_parts
        else np.empty((0, 4))
    )
    if config.get("widths_from_walls"):
        widths = widths_inside_walls(points, widths, walls)
        metadata["half_width"] = np.round(widths, 3).tolist()
    patches = bridge_road_seams(road, config.get("collision_seams", []))
    if len(patches):
        triangles = np.concatenate([triangles, patches])
        surfaces = np.r_[surfaces, np.zeros(len(patches), dtype="u1")]
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
        "seam_triangles": len(patches),
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
