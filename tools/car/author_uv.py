"""Author Monoposto correspondence on FP04RB's original UV charts, before modifiers.

Coordinates in the JSON are original Blender UVs and official-guide PNG UVs.
The runtime atlas is independent. No world-position classification is used.
"""

import hashlib
import json
from pathlib import Path

import numpy as np

PROFILE = Path(__file__).with_name("monoposto-modern.json")
LAYER = "Monoposto2026"


def strip(points, rows):
    """Interpolate an authored UV cage. Each row has source U, two V boundaries,
    and the corresponding two guide landmarks. Shared boundaries stay continuous.
    """
    rows = np.asarray(rows, dtype=float)
    u, v = points.T
    lo = np.interp(u, rows[:, 0], rows[:, 1])
    hi = np.interp(u, rows[:, 0], rows[:, 2])
    t = (v - lo) / np.maximum(hi - lo, 1e-9)
    a = np.stack([np.interp(u, rows[:, 0], rows[:, i]) for i in (3, 4)], axis=-1)
    b = np.stack([np.interp(u, rows[:, 0], rows[:, i]) for i in (5, 6)], axis=-1)
    return a + np.clip(t, 0, 1)[:, None] * (b - a)


def charts(mesh):
    """Components share original UV edges, rather than merely touching in 3D."""
    uv = mesh.uv_layers.active.data
    edges = {}
    neighbours = [set() for _ in mesh.polygons]
    for face in mesh.polygons:
        loops = list(face.loop_indices)
        for a, b in zip(loops, loops[1:] + loops[:1]):
            key = tuple(
                sorted(
                    (
                        tuple(round(x, 6) for x in uv[a].uv),
                        tuple(round(x, 6) for x in uv[b].uv),
                    )
                )
            )
            if key in edges:
                other = edges[key]
                neighbours[other].add(face.index)
                neighbours[face.index].add(other)
            else:
                edges[key] = face.index
    remaining = set(range(len(mesh.polygons)))
    while remaining:
        seed = remaining.pop()
        group = [seed]
        todo = [seed]
        while todo:
            for other in neighbours[todo.pop()]:
                if other in remaining:
                    remaining.remove(other)
                    group.append(other)
                    todo.append(other)
        yield group


def author(objects, output):
    profile = json.loads(PROFILE.read_text())
    report = []
    # Do not leave obsolete painted-chart evidence when a panel becomes carbon.
    for name in ('body_top_wing', 'body_side_top_wings'):
        if name not in profile['objects']:
            (output / (name + '-charts.npz')).unlink(missing_ok=True)
    for obj in objects:
        specs = profile["objects"].get(obj.name)
        if specs is None:
            continue
        mesh = obj.data
        original = mesh.uv_layers.active
        groups = list(charts(mesh))
        layer = mesh.uv_layers.new(name=LAYER)
        for loop in layer.data:
            loop.uv = (-1, -1)
        found = []
        for group in groups:
            loops = [i for f in group for i in mesh.polygons[f].loop_indices]
            points = np.array([original.data[i].uv[:] for i in loops])
            bounds = np.array([points.min(axis=0), points.max(axis=0)])
            matches = [s for s in specs if np.max(np.abs(bounds - s["bounds"])) < 0.003]
            if len(matches) > 1:
                raise ValueError("Ambiguous original UV chart: " + obj.name)
            if not matches:
                continue  # Original unpainted microcharts retain carbon.
            spec = matches[0]
            found.append(spec["panel"])
            parts = spec.get("parts", [spec])
            patch_quality = []
            mesh.calc_loop_triangles()
            for part in parts:
                selected = []
                for face_id in group:
                    ids = list(mesh.polygons[face_id].loop_indices)
                    p = np.array([original.data[i].uv[:] for i in ids])
                    if spec.get("swap"):
                        p = p[:, ::-1]
                    c = p.mean(axis=0)
                    if (
                        part.get("u", [-2, 2])[0] <= c[0] <= part.get("u", [-2, 2])[1]
                        and part.get("v", [-2, 2])[0]
                        <= c[1]
                        <= part.get("v", [-2, 2])[1]
                    ):
                        selected.append(face_id)
                if not selected:
                    continue
                # Existing thin wing charts already have straight, nonfolding UVs.
                if obj.name == "body_object" or part.get("boundary", spec.get("boundary")):
                    mapped = boundary_transfer(
                        mesh,
                        original,
                        selected,
                        part["rows"],
                        part.get("swap", spec.get("swap", False)),
                        part.get("reverse", False),
                        part.get("flip", False),
                        part.get("interior", spec.get("interior")),
                    )
                else:
                    ids = [i for f in selected for i in mesh.polygons[f].loop_indices]
                    p = np.array([original.data[i].uv[:] for i in ids])
                    if spec.get("swap"):
                        p = p[:, ::-1]
                    mapped = dict(zip(ids, strip(p, part["rows"])))
                determinants = []
                collapsed = 0
                chosen = set(selected)
                for triangle in mesh.loop_triangles:
                    if triangle.polygon_index not in chosen:
                        continue
                    old = np.array([original.data[i].uv[:] for i in triangle.loops])
                    new = np.array([mapped[i] for i in triangle.loops])

                    def det(p):
                        a, b = p[1] - p[0], p[2] - p[0]
                        return a[0] * b[1] - a[1] * b[0]

                    before, after = det(old), det(new)
                    if abs(before) < 1e-10:
                        continue
                    if abs(after) < 1e-10:
                        collapsed += 1
                    else:
                        determinants.append(before * after > 0)
                positive = int(sum(determinants))
                negative = len(determinants) - positive
                patch_quality.append(
                    dict(
                        faces=len(selected),
                        collapsed=collapsed,
                        folded=min(positive, negative),
                        triangles=len(determinants),
                    )
                )
                for i, value in mapped.items():
                    if "solid" in spec:
                        value = spec["solid"]
                    u = 1 - value[0] if spec.get("mirror") else value[0]
                    layer.data[i].uv = (u, 1 - value[1])
            report.append(
                {
                    "object": obj.name,
                    "panel": spec["panel"],
                    "faces": len(group),
                    "original_bounds": bounds.tolist(),
                    "patch_quality": patch_quality,
                }
            )
        missing = [s["panel"] for s in specs if s["panel"] not in found]
        if missing:
            raise ValueError(
                "Original mesh UV charts differ from profile: "
                + obj.name
                + str(missing)
            )
        mesh.calc_loop_triangles()
        original_triangles = np.array(
            [[original.data[i].uv[:] for i in t.loops] for t in mesh.loop_triangles]
        )
        mapped_triangles = np.array(
            [[layer.data[i].uv[:] for i in t.loops] for t in mesh.loop_triangles]
        )
        output.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(
            output / (obj.name + "-charts.npz"),
            original=original_triangles,
            mapped=mapped_triangles,
        )
        mesh.uv_layers.active = original
    output.mkdir(parents=True, exist_ok=True)
    (output / "correspondence.json").write_text(
        json.dumps(
            {
                "profile_sha256": hashlib.sha256(PROFILE.read_bytes()).hexdigest(),
                "charts": report,
            },
            indent=2,
        )
        + "\n"
    )
    print("AUTHORED UV CHARTS", len(report), flush=True)


def boundary_transfer(
    mesh, original, faces, rows, swap=False, reverse=False, flip=False, interior=None
):
    """Transfer measured chart sections into the authored guide boundary.

    Source bounds come from the actual panel boundary edges at each loop U.
    The monotonic longitudinal parameter preserves source seams and holes.
    """
    edges = {}
    points = {}
    for f in faces:
        loops = list(mesh.polygons[f].loop_indices)
        for i in loops:
            p = tuple(original.data[i].uv)
            points[i] = p[::-1] if swap else p
        for a, b in zip(loops, loops[1:] + loops[:1]):
            key = tuple(sorted((points[a], points[b])))
            edges[key] = edges.get(key, 0) + 1
    boundary = np.array(list(edges))
    if not len(boundary):
        raise ValueError("Source chart has no boundary")
    start, end = boundary[:, 0], boundary[:, 1]
    low_u = min(p[0] for p in points.values())
    high_u = max(p[0] for p in points.values())
    low_v = min(p[1] for p in points.values())
    high_v = max(p[1] for p in points.values())
    rows = np.array(rows)
    cache = {}
    mapped = {}
    for loop, (u, v) in points.items():
        if u not in cache:
            hit = (np.minimum(start[:, 0], end[:, 0]) <= u + 1e-9) & (
                np.maximum(start[:, 0], end[:, 0]) >= u - 1e-9
            )
            a, b = start[hit], end[hit]
            span = b[:, 0] - a[:, 0]
            regular = np.abs(span) > 1e-10
            values = list(
                a[regular, 1]
                + (u - a[regular, 0]) / span[regular] * (b[regular, 1] - a[regular, 1])
            )
            values.extend(a[~regular, 1])
            values.extend(b[~regular, 1])
            cache[u] = (min(values), max(values))
        lo, hi = cache[u]
        t = (v - lo) / max(hi - lo, 1e-9)
        fraction = (u - low_u) / max(high_u - low_u, 1e-9)
        if interior:
            fraction = np.interp(fraction, [0, .16, .84, 1], [0, .04, .96, 1])
        along = rows[0, 0] + fraction * (
            rows[-1, 0] - rows[0, 0]
        )
        if reverse:
            along = rows[0, 0] + rows[-1, 0] - along
        A = np.array([np.interp(along, rows[:, 0], rows[:, i]) for i in (3, 4)])
        B = np.array([np.interp(along, rows[:, 0], rows[:, i]) for i in (5, 6)])
        if interior:
            # Keep a rectangular logo field inside the measured notched chart.
            # Only the outer margin follows its silhouette; letters no longer
            # shear whenever a section crosses the endplate's missing corner.
            c, d = low_v + np.array(interior["source_v"]) * (high_v - low_v)
            weight = np.interp((u-low_u)/(high_u-low_u),
                               [0, .16, .84, 1], [0, 1, 1, 0])
            target = np.array(interior["guide_v"])
            inner = target
            direction = np.sign(B[1]-A[1])
            if direction < 0:
                inner = inner[::-1]
            # Contour corners can approach the field at the extreme sections.
            # The field blends into those corners without reversing a section.
            ia = A[1] + .25*(B[1]-A[1])
            ib = A[1] + .75*(B[1]-A[1])
            inner = np.array([ia, ib])*(1-weight) + inner*weight
            fractions = np.clip((inner-A[1])/(B[1]-A[1]), .02, .98)
            c = np.clip(c, lo+.1*(hi-lo), lo+.45*(hi-lo))
            d = np.clip(d, lo+.55*(hi-lo), hi-.1*(hi-lo))
            t = np.interp(v, [lo, c, d, hi], [0, *fractions, 1])
        mapped[loop] = B + t * (A - B) if flip else A + t * (B - A)
    return mapped
