"""Convert the Speed Dreams "Ardennen-Spa" track into game assets.

Inputs: a checkout/extract of https://codeberg.org/leillo1975/ardennen-spa
Outputs (in the destination directory):
  visual.glb      render geometry, one mesh per material and 512 m tile
  collision.bin   drivable triangles with surface ids + 2D wall segments
  track.json      centerline, widths, sector/corner names and metadata

Usage: python3 -I tools/track/convert_spa.py <ardennen-spa dir> <out dir>
"""

from __future__ import annotations

import json
import math
import os
import re
import sys
import xml.etree.ElementTree as ElementTree

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ac3d import read_ac  # noqa: E402
from collision import (  # noqa: E402
    SURFACE_ASPHALT,
    SURFACE_CONCRETE,
    SURFACE_GRASS,
    SURFACE_GRAVEL,
    SURFACE_KERB,
    write_collision,
)
from glb import GlbMaterial, GlbMesh, write_glb  # noqa: E402

TILE_SIZE = 512.0
CREASE_COS = math.cos(math.radians(60.0))

# Speed Dreams track generator object families.
DRIVABLE_PREFIXES = ("tkMn", "tkLS", "tkRS", "t0LB", "t0RB", "t1LB", "t1RB", "t2RB", "TERR")
BARRIER_PREFIXES = ("B0Lt", "B0Rt", "B1Lt", "B1Rt", "B2Lt", "B2Rt")


def prefix(name: str) -> str:
    return name.rstrip("0123456789")


def surface_for_texture(texture: str | None) -> int:
    name = (texture or "").lower()
    if "curb" in name:
        return SURFACE_KERB
    if "gravel" in name or "sand" in name:
        return SURFACE_GRAVEL
    if "grass" in name:
        return SURFACE_GRASS
    if "concrete" in name:
        return SURFACE_CONCRETE
    return SURFACE_ASPHALT


def face_normals(vertices: np.ndarray, triangles: np.ndarray) -> np.ndarray:
    a, b, c = (vertices[triangles[:, i]] for i in range(3))
    normals = np.cross(b - a, c - a)
    length = np.linalg.norm(normals, axis=1, keepdims=True)
    return normals / np.maximum(length, 1e-12)


def corner_normals(obj, normals: np.ndarray) -> np.ndarray:
    """Per-corner normals: smoothed across faces within the crease angle."""
    result = np.repeat(normals[:, None, :], 3, axis=1)
    smooth = np.nonzero(obj.smooth)[0]
    if smooth.size == 0:
        return result
    faces_by_vertex: dict[int, list[int]] = {}
    for face in smooth:
        for vertex in obj.triangles[face]:
            faces_by_vertex.setdefault(int(vertex), []).append(int(face))
    for face in smooth:
        own = normals[face]
        for corner, vertex in enumerate(obj.triangles[face]):
            shared = faces_by_vertex[int(vertex)]
            candidates = normals[shared]
            mask = candidates @ own > CREASE_COS
            total = candidates[mask].sum(axis=0)
            length = np.linalg.norm(total)
            if length > 1e-9:
                result[face, corner] = total / length
    return result


def texture_has_cutout(image: Image.Image) -> bool:
    if image.mode != "RGBA":
        return False
    alpha = np.asarray(image.getchannel("A"))
    return float((alpha < 128).mean()) > 0.01


def load_texture(source: str, name: str, cache: dict) -> Image.Image | None:
    if name in cache:
        return cache[name]
    path = os.path.join(source, name)
    image = None
    if os.path.exists(path):
        image = Image.open(path)
        image = image.convert("RGBA") if "A" in image.getbands() or image.mode == "P" else image.convert("RGB")
        if image.mode == "RGBA" and not texture_has_cutout(image):
            image = image.convert("RGB")
    else:
        print(f"warning: missing texture {name}", file=sys.stderr)
    cache[name] = image
    return image


def extract_centerline(objects) -> tuple[np.ndarray, np.ndarray]:
    """Centerline and half-widths from the tkMn strips (vertex pairs span the road)."""
    points: list[np.ndarray] = []
    widths: list[float] = []
    for obj in objects:
        if prefix(obj.name) != "tkMn":
            continue
        verts = obj.vertices
        for i in range(0, len(verts) - 1, 2):
            a, b = verts[i], verts[i + 1]
            middle = (a + b) * 0.5
            if points and np.linalg.norm(points[-1] - middle) < 0.25:
                continue
            points.append(middle)
            widths.append(float(np.linalg.norm(b - a)) * 0.5)
    centerline = np.array(points)
    if np.linalg.norm(centerline[0] - centerline[-1]) < 0.25:
        centerline = centerline[:-1]
        widths = widths[:-1]
    return centerline, np.array(widths)


def segment_names(xml_path: str) -> list[tuple[str, float]]:
    """Named corners with their distance from the start line, scaled later."""
    with open(xml_path, encoding="utf-8") as handle:
        text = handle.read()
    # Speed Dreams includes shared surface tables through external entities.
    text = re.sub(r"&[A-Za-z][\w.-]*;", "", re.sub(r"<!DOCTYPE[^\[>]*(\[.*?\])?>", "", text, flags=re.S))
    root = ElementTree.fromstring(text)
    segments = None
    for section in root.iter("section"):
        if section.get("name") == "Track Segments":
            segments = section
            break
    if segments is None:
        return []
    named: list[tuple[str, float]] = []
    distance = 0.0
    for segment in segments.findall("section"):
        values = {
            attr.get("name"): attr.get("val")
            for attr in segment.findall("attnum") + segment.findall("attstr")
        }
        name = segment.get("name")
        if values.get("type") == "str":
            length = float(values.get("lg", 0.0))
        else:
            arc = math.radians(float(values.get("arc", 0.0)))
            start = float(values.get("radius", 0.0))
            end = float(values.get("end radius", start))
            length = arc * (start + end) * 0.5
        lowered = name.lower()
        if not (lowered.startswith("straight") or lowered.startswith("curve")
                or "pits" in lowered or lowered == "goal"):
            named.append((name, distance))
        distance += length
    return [(name, start / distance) for name, start in named]


def wall_segments(objects) -> np.ndarray:
    segments: set[tuple[float, float, float, float]] = set()
    for obj in objects:
        family = prefix(obj.name)
        if family not in DRIVABLE_PREFIXES and family not in BARRIER_PREFIXES:
            continue
        normals = face_normals(obj.vertices, obj.triangles)
        for face, normal in zip(obj.triangles, normals):
            if abs(normal[1]) > 0.5:
                continue
            corners = obj.vertices[face]
            if corners[:, 1].max() - corners[:, 1].min() < 0.4:
                continue  # kerb lips and small steps are driven over, not walls
            for i in range(3):
                a, b = corners[i], corners[(i + 1) % 3]
                if math.hypot(a[0] - b[0], a[2] - b[2]) < 0.05:
                    continue
                key = tuple(round(v, 2) for v in (a[0], a[2], b[0], b[2]))
                reverse = (key[2], key[3], key[0], key[1])
                if reverse not in segments:
                    segments.add(key)
    return np.array(sorted(segments), dtype=np.float32).reshape(-1, 4)


def main() -> None:
    source, destination = sys.argv[1], sys.argv[2]
    os.makedirs(destination, exist_ok=True)
    ac = read_ac(os.path.join(source, "ardennen-spa.ac"))
    print(f"read {len(ac.objects)} polygon objects")

    texture_cache: dict[str, Image.Image | None] = {}
    materials: list[GlbMaterial] = []
    material_index: dict[str, int] = {}
    buckets: dict[tuple[int, int, int], list[tuple[np.ndarray, np.ndarray, np.ndarray]]] = {}

    drivable_positions: list[np.ndarray] = []
    drivable_surfaces: list[np.ndarray] = []

    for obj in ac.objects:
        family = prefix(obj.name)
        key = obj.texture or "untextured"
        if key not in material_index:
            image = load_texture(source, obj.texture, texture_cache) if obj.texture else None
            ac_material = ac.materials[int(obj.material[0])] if ac.materials else None
            color = (1.0, 1.0, 1.0, 1.0)
            if image is None and ac_material is not None:
                color = (*ac_material.rgb, 1.0)
            name = os.path.splitext(key)[0]
            materials.append(
                GlbMaterial(
                    name=name,
                    base_color=color,
                    image=image,
                    alpha_mask=image is not None and image.mode == "RGBA",
                    roughness=0.55 if surface_for_texture(key) == SURFACE_ASPHALT else 0.85,
                )
            )
            material_index[key] = len(materials) - 1
        material = material_index[key]

        normals = face_normals(obj.vertices, obj.triangles)
        corners = obj.vertices[obj.triangles]  # (t, 3, 3)
        per_corner_normals = corner_normals(obj, normals)
        centers = corners.mean(axis=1)
        tiles = np.floor(centers[:, [0, 2]] / TILE_SIZE).astype(np.int64)
        for tile in np.unique(tiles, axis=0):
            mask = np.all(tiles == tile, axis=1)
            buckets.setdefault((material, int(tile[0]), int(tile[1])), []).append(
                (corners[mask], per_corner_normals[mask], obj.uvs[mask])
            )

        if family in DRIVABLE_PREFIXES:
            flat = normals[:, 1] > 0.6
            if flat.any():
                drivable_positions.append(corners[flat].astype(np.float32))
                drivable_surfaces.append(
                    np.full(int(flat.sum()), surface_for_texture(obj.texture), dtype=np.uint8)
                )

    meshes: list[GlbMesh] = []
    for (material, tile_x, tile_z), parts in sorted(buckets.items()):
        corners = np.concatenate([p[0] for p in parts]).reshape(-1, 3)
        normals = np.concatenate([p[1] for p in parts]).reshape(-1, 3)
        uvs = np.concatenate([p[2] for p in parts]).reshape(-1, 2)
        packed = np.concatenate([corners, normals, uvs], axis=1).astype(np.float32)
        unique, inverse = np.unique(packed, axis=0, return_inverse=True)
        meshes.append(
            GlbMesh(
                name=f"{materials[material].name}@{tile_x}_{tile_z}",
                material=material,
                positions=unique[:, 0:3],
                normals=unique[:, 3:6],
                uvs=unique[:, 6:8],
                indices=inverse.reshape(-1).astype(np.uint32),
            )
        )
    write_glb(os.path.join(destination, "visual.glb"), materials, meshes)
    print(f"wrote visual.glb: {len(materials)} materials, {len(meshes)} meshes")

    walls = wall_segments(ac.objects)
    write_collision(
        os.path.join(destination, "collision.bin"),
        np.concatenate(drivable_positions),
        np.concatenate(drivable_surfaces),
        walls,
    )
    print(f"wrote collision.bin: {sum(len(s) for s in drivable_surfaces)} triangles, {len(walls)} walls")

    centerline, half_widths = extract_centerline(ac.objects)
    steps = np.linalg.norm(np.roll(centerline, -1, axis=0) - centerline, axis=1)
    length = float(steps.sum())
    corners = [
        {"name": name, "distance": round(fraction * length, 1)}
        for name, fraction in segment_names(os.path.join(source, "ardennen-spa.xml"))
    ]
    track = {
        "name": "Spa-Francorchamps",
        "source": "Ardennen-Spa Reworked v0.9.2 by Leandro Vergara (leillo1975), CC BY 4.0",
        "length": round(length, 2),
        "centerline": np.round(centerline, 3).tolist(),
        "half_width": np.round(half_widths, 3).tolist(),
        "corners": corners,
        "sun_direction": [-0.35, -0.78, -0.52],
    }
    with open(os.path.join(destination, "track.json"), "w") as handle:
        json.dump(track, handle, separators=(",", ":"))
    print(f"wrote track.json: {len(centerline)} centerline points, {length:.0f} m, {len(corners)} named corners")


if __name__ == "__main__":
    main()
