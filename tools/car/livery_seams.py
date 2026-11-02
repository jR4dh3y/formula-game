"""Separate paint charts before collapse decimation can mix their UVs."""

import bmesh
from pathlib import Path

import numpy as np
from scipy import ndimage

_interiors = {}


def constrain(obj):
    """Restore each face's authored panel after subdivision or collapse."""
    mesh = obj.data
    attribute = mesh.attributes.get("livery_2025_panel")
    layer = mesh.uv_layers.get("livery")
    if attribute is None or layer is None:
        return
    labels = np.load(
        Path(__file__).resolve().parents[2]
        / "raw/gen22/livery-mapping/2025/islands.npy"
    )
    owners = np.empty(len(mesh.polygons), dtype=np.int32)
    attribute.data.foreach_get("value", owners)
    paint = {
        i
        for i, material in enumerate(mesh.materials)
        if material
        and (
            material.name.startswith("source_paint_")
            or material.name == "Direct template sheet"
        )
    }
    for face in mesh.polygons:
        if face.material_index not in paint:
            owners[face.index] = -1
    owners = np.repeat(owners, [face.loop_total for face in mesh.polygons])
    uv = np.empty(len(mesh.loops) * 2, dtype=np.float32)
    layer.data.foreach_get("uv", uv)
    pixels = uv.reshape(-1, 2) * [2048, -2048] + [0, 2048]
    for panel in np.unique(owners):
        if panel < 0:
            continue
        take = owners == panel
        if panel == 0:
            pixels[take] = 0
            continue
        if panel not in _interiors:
            rows, columns = np.nonzero(labels == panel)
            origin = np.array([columns.min(), rows.min()])
            mask = (
                labels[rows.min() : rows.max() + 1, columns.min() : columns.max() + 1]
                == panel
            )
            mask = ndimage.binary_erosion(mask, iterations=3)
            if not mask.any():
                raise ValueError(f"empty template panel {panel}")
            _, nearest = ndimage.distance_transform_edt(~mask, return_indices=True)
            _interiors[panel] = origin, mask, nearest
        origin, mask, nearest = _interiors[panel]
        xy = np.clip(
            np.rint(pixels[take]).astype(int) - origin,
            0,
            np.array(mask.shape[::-1]) - 1,
        )
        outside = ~mask[xy[:, 1], xy[:, 0]]
        ids = np.flatnonzero(take)[outside]
        xy = xy[outside]
        pixels[ids] = origin + np.column_stack(
            (nearest[1, xy[:, 1], xy[:, 0]], nearest[0, xy[:, 1], xy[:, 0]])
        )
    layer.data.foreach_set(
        "uv", (pixels / [2048, -2048] + [0, 1]).astype(np.float32).ravel()
    )


def split(obj):
    mesh = obj.data
    if not mesh.uv_layers.get("livery"):
        return
    paint = {
        i
        for i, material in enumerate(mesh.materials)
        if material and material.name.startswith("source_paint_")
    }
    if not paint:
        return
    bm = bmesh.new()
    bm.from_mesh(mesh)
    uv = bm.loops.layers.uv["livery"]
    edges = []
    for edge in bm.edges:
        if len(edge.link_loops) != 2 or not any(
            face.material_index in paint for face in edge.link_faces
        ):
            continue
        a, b = edge.link_loops
        if (a[uv].uv - b.link_loop_next[uv].uv).length > 1e-6 or (
            b[uv].uv - a.link_loop_next[uv].uv
        ).length > 1e-6:
            edges.append(edge)
    if not edges:
        bm.free()
        return
    # Splitting a smooth panel must not introduce a hard shading seam.
    normal = bm.loops.layers.float_vector.new("livery_corner_normal")
    for face in bm.faces:
        for loop, index in zip(face.loops, mesh.polygons[face.index].loop_indices):
            loop[normal] = mesh.corner_normals[index].vector
    bmesh.ops.split_edges(bm, edges=edges)
    normals = [tuple(loop[normal]) for face in bm.faces for loop in face.loops]
    bm.loops.layers.float_vector.remove(normal)
    bm.to_mesh(mesh)
    bm.free()
    mesh.normals_split_custom_set(normals)
    print("SPLIT LIVERY SEAMS", obj.name, len(edges), flush=True)
