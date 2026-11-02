"""Direct 2025 template coordinates on FP04RB's physical paint panels.

All coordinates are pixels in the original 2048 square sheet. UV0 is read only.
The recorded anatomical loops select panels, then solve onto 2025 islands.
"""

import numpy as np
import json
from pathlib import Path
from scipy import ndimage as ndi
from scipy.spatial import cKDTree

import author_template_uv as base

CARBON = np.array([0.0, 0.0])


def panel(template, island, span, chord, *, vertical=False, bounds=None):
    """Map into an island contour with a two-pixel interior gutter."""
    mask = template.labels == island
    if bounds is not None:
        x0, y0, x1, y1 = bounds
        keep = np.zeros_like(mask)
        keep[y0:y1, x0:x1] = True
        mask &= keep
    mask = ndi.binary_erosion(mask, iterations=2)
    if vertical:
        mask = mask.T
    columns = np.flatnonzero(mask.any(0))
    if not len(columns):
        raise ValueError(f"empty template island {island}: {bounds}")
    low = np.argmax(mask[:, columns], axis=0)
    high = len(mask) - 1 - np.argmax(mask[::-1, columns], axis=0)
    x = columns[0] + 2 + np.clip(span, 0, 1) * (np.ptp(columns) - 4)
    a = np.interp(x, columns, low) + 2
    b = np.interp(x, columns, high) - 2
    y = a + np.clip(chord, 0, 1) * (b - a)
    return np.column_stack((y, x) if vertical else (x, y))


def body(pos, loops, uv, triangles, faces, starts, normals, template):
    import build_map_v3 as solver

    mesh = base.PanelMesh(pos, loops, uv, triangles)
    spec = json.loads(Path(__file__).with_name("patches_2026.json").read_text())
    anchors = {
        "engine_cover_R": [[885, 23], [90, 23], [155, 315], [1005, 345]],
        "sidepod_R": [
            [650, 1530],
            [520, 1400],
            [615, 1180],
            [1050, 1160],
            [1400, 1305],
        ],
    }
    coordinates = []
    vertex_owners = np.full(len(pos), -1)
    for patch in spec["patches"][:2]:
        for left in (False, True):
            target = iter(anchors[patch["name"]])
            loop = []
            for point in patch["loop"]:
                node = mesh.node_from_uv(point["uv"])
                xyz = mesh.P[node] * [1, 1, -1 if left else 1]
                entry = {"xyz": xyz.tolist(), "edge": point.get("edge", "line")}
                if "t" in point:
                    tx, ty = next(target)
                    if patch["name"].startswith("engine"):
                        if not left:
                            tx = 2069 - tx
                    elif left:
                        ty = 3138 - ty
                    entry["t"] = [tx, ty]
                loop.append(entry)
            island = (
                (1 if left else 2)
                if patch["name"].startswith("engine")
                else (34 if left else 25)
            )
            seed = mesh.P[mesh.node_from_uv(patch["seed"]["uv"])] * [
                1,
                1,
                -1 if left else 1,
            ]
            adapted = dict(
                patch,
                mirror=False,
                loop=loop,
                seed={"xyz": seed.tolist()},
                island=island,
            )
            adapted.pop("interior", None)
            shape = solver.IslandShape(template.labels, island)
            solved = solver.solve_patch(mesh, adapted, shape, 2048)
            constrain_panel_metric(mesh, solved, patch["name"], left, shape, solver)
            coords = np.full((len(pos), 2), np.nan)
            coords[solved["pn"]] = shape.clamp(solved["xy"])[0]
            vertex_owners[solved["pn"]] = len(coordinates)
            coordinates.append(coords)
    labels = base.charts(loops, uv, triangles, starts)
    result = np.tile(CARBON, (len(loops), 1))
    trees = []
    for coords in coordinates:
        vertices = np.flatnonzero(np.isfinite(coords[:, 0]))
        trees.append((vertices, cKDTree(pos[vertices])))
    for face, start in enumerate(starts):
        end = starts[face + 1] if face + 1 < len(starts) else len(loops)
        ids = np.arange(start, end)
        votes = vertex_owners[loops[ids]]
        votes = votes[votes >= 0]
        if len(votes):
            owner = int(np.bincount(votes).argmax())
        elif labels[face] in [0, 1, 2, 7, 8] and pos[loops[ids], 1].mean() >= 2.32:
            owner = int(pos[loops[ids], 0].mean() > 0.0598)
        else:
            continue
        coords = coordinates[owner]
        vertices, tree = trees[owner]
        v = loops[ids]
        missing = ~np.isfinite(coords[v, 0])
        v[missing] = vertices[tree.query(pos[v[missing]])[1]]
        result[ids] = coords[v]
    return finish_body(pos, loops, faces, starts, labels, result, template)


def constrain_panel_metric(mesh, solved, name, left, shape, solver):
    """Keep the broad side of each panel undistorted in a side projection.

    Sparse landmark pins let the cover fin and sidepod shoulder shear nearby
    artwork. A small planar region fixes that metric; the surrounding mesh
    still solves harmonically to the anatomical panel boundary.
    """
    nodes = solved["pn"]
    xyz = mesh.P[nodes]
    if name.startswith("engine"):
        centre = np.array([-3.05, 0.86])
        radius = np.array([0.43, 0.24])
        target = (xyz[:, :2] - centre) * [-450, -450] + [330, 140]
        if not left:
            target[:, 0] = 2069 - target[:, 0]
        outside_spine = np.abs(xyz[:, 2]) > 0.035
    else:
        centre = np.array([-2.08, 0.50])
        target = (xyz[:, :2] - centre) * [-300, 300] + [840, 1342]
        if left:
            target[:, 1] = 3138 - target[:, 1]
        centre = np.array([-2.65, 0.43])
        radius = np.array([0.85, 0.12])
        outside_spine = np.abs(xyz[:, 2]) > 0.25
    inside = np.sum(((xyz[:, :2] - centre) / radius) ** 2, axis=1) < 1
    if not name.startswith("engine"):
        inside |= np.sum(((xyz[:, :2] - [-2.08, 0.50]) / [0.18, 0.13]) ** 2, axis=1) < 1
    fixed = np.isin(nodes, solved["loop"])
    metric = inside & outside_spine & ~fixed
    values = solved["xy"].copy()
    values[metric] = shape.clamp(target[metric])[0]
    fixed |= metric
    solved["xy"] = solver.harmonic(mesh, nodes, nodes[fixed], values[fixed])


def finish_body(pos, loops, faces, starts, labels, result, template):
    p = pos[loops]
    x, y, z = p.T
    centres = np.array(
        [pos[loops[a:b]].mean(0) for a, b in zip(starts, np.r_[starts[1:], len(loops)])]
    )
    fx, fy, fz = centres.T
    main = np.isin(labels, [0, 1, 2])
    nose = main & (fy < 1.45)
    ids = np.flatnonzero(nose[faces])
    # Nose points down the sheet. Circumferential width includes chassis sides.
    rear = np.interp(
        y[ids], [-0.708, -0.3, 0.2, 0.75, 1.25, 1.45], [1, 0.83, 0.66, 0.45, 0.18, 0]
    )
    width = np.interp(
        y[ids], [-0.708, -0.3, 0.3, 0.8, 1.45], [0.085, 0.105, 0.125, 0.145, 0.28]
    )
    result[ids] = panel(
        template, 27, rear, 0.5 + (x[ids] - 0.0598) / (2 * width), vertical=True
    )
    rails = (
        main & (fy >= 1.45) & (fy < 2.34) & (fz > 0.6) & (np.abs(fx - 0.0598) < 0.30)
    )
    ids = np.flatnonzero(rails[faces])
    # C-shaped cockpit rim opens towards the rear on the template.
    yy = np.clip((y[ids] - 1.45) / 0.89, 0, 1)
    result[ids] = panel(template, 18, yy, np.where(fx[faces[ids]] > 0.0598, 0.05, 0.95))
    halo = np.isin(labels, [3, 4, 5])
    ids = np.flatnonzero(halo[faces])
    hp = p[ids]
    # Unroll the halo circumference independently of the two arms' length.
    phi = np.arctan2((hp[:, 0] - 0.0598) * 2, 2.31 - hp[:, 1])
    q = np.hypot((hp[:, 0] - 0.0598) * 2, 2.31 - hp[:, 1])
    bins = np.linspace(-np.pi / 2, np.pi / 2, 81)
    station = np.clip(np.digitize(phi, bins) - 1, 0, len(bins) - 2)
    qc, zc = [], []
    for k in range(len(bins) - 1):
        take = station == k
        qc.append((q[take].min() + q[take].max()) / 2 if take.any() else np.nan)
        zc.append((hp[take, 2].min() + hp[take, 2].max()) / 2 if take.any() else np.nan)
    for values in (qc, zc):
        good = np.flatnonzero(np.isfinite(values))
        values[:] = np.interp(np.arange(len(values)), good, np.array(values)[good])
    theta = np.arctan2(q - np.array(qc)[station], hp[:, 2] - np.array(zc)[station])
    circumference = 0.5 + theta / (2 * np.pi)
    for left in (False, True):
        take = ((centres[:, 0] > 0.0598)[faces[ids]]) == left
        bounds = (1325, 1569, 1914, 1885) if left else (1325, 1235, 1914, 1568)
        result[ids[take]] = panel(
            template,
            31,
            np.abs(phi[take]) / (np.pi / 2),
            circumference[take] if left else 1 - circumference[take],
            bounds=bounds,
        )
    return result


def front(pos, loops, uv, triangles, faces, starts, normals, template):
    result = np.tile(CARBON, (len(loops), 1))
    labels = base.charts(loops, uv, triangles, starts)
    for label in np.unique(labels):
        selected = labels == label
        ids = np.flatnonzero(selected[faces])
        p = pos[loops[ids]]
        n = np.median(normals[selected], axis=0)
        if abs(n[0]) > 0.75:
            right = p[:, 0].mean() < 0.05
            outer = (n[0] < 0) == right
            # Small UniCredit faces belong to the front endplates. The B&O
            # panels (6/7) belong to the rear wing, as the photos confirm.
            island = (11 if right else 4) if outer else (23 if right else 21)
            span = (p[:, 1] + 0.42) / 0.50
            down = (0.49 - p[:, 2]) / 0.39
            result[ids] = panel(
                template, island, 1 - span if right == outer else span, down
            )
        elif label in (3, 6, 14, 15, 20, 21):
            # The template has four distinct ribbons. Keep each physical flap
            # inside its own ribbon instead of interpolating across the slots.
            rank = (
                0
                if label in (20, 21)
                else 1
                if label in (14, 15)
                else 2
                if label == 6
                else 3
            )
            selected &= normals[:, 2] > 0.15
            ids = np.flatnonzero(selected[faces])
            p = pos[loops[ids]]
            _, chord = base.profile_coordinates(
                pos, loops, triangles, faces, selected, 0, 1
            )
            span = np.clip((p[:, 0] + 0.9545) / 2, 0, 1)
            if rank > 0:
                chord = np.clip(chord / (0.62 if rank == 1 else 0.58), 0, 1)
            result[ids] = wing_ribbon(template, span, 1 - chord, rank)
    return result


def wing_ribbon(template, span, chord, rank):
    """Four curved front-wing ribbons share one connected outline, island 19."""
    x = 1328 + np.clip(span, 0, 1) * 708
    # The inner ends of the upper two flaps stop at the nose. Coordinates in
    # this covered portion extend the nearest printed border, not another flap.
    if rank < 3:
        x = np.where(x < 1705, np.minimum(x, 1600), np.maximum(x, 1790))
    mask = template.labels == 19
    columns, lower, upper = [], [], []
    for col in np.flatnonzero(mask.any(0)):
        edges = np.flatnonzero(np.diff(np.r_[False, mask[:, col], False])).reshape(
            -1, 2
        )
        edges = edges[np.diff(edges, axis=1).ravel() >= 10]
        if len(edges) == 4:
            columns.append(col)
            lower.append(edges[rank, 0])
            upper.append(edges[rank, 1] - 1)
    a = np.interp(x, columns, lower)
    b = np.interp(x, columns, upper)
    y = a + 3 + np.clip(chord, 0, 1) * np.maximum(b - a - 6, 1)
    # Clamp to this island only, never to a neighbouring component.
    _, nearest = ndi.distance_transform_edt(~mask, return_indices=True)
    ix, iy = np.clip(x.astype(int), 0, 2047), np.clip(y.astype(int), 0, 2047)
    outside = ~mask[iy, ix]
    x[outside] = nearest[1, iy[outside], ix[outside]]
    y[outside] = nearest[0, iy[outside], ix[outside]]
    return np.column_stack((x, y))


def rear(pos, loops, uv, triangles, faces, starts, normals, template, drs):
    result = np.tile(CARBON, (len(loops), 1))
    labels = base.charts(loops, uv, triangles, starts)
    for label in np.unique(labels):
        sel = labels == label
        ids = np.flatnonzero(sel[faces])
        p = pos[loops[ids]]
        n = np.median(normals[sel], axis=0)
        if abs(n[0]) > 0.6 and not drs:
            right = p[:, 0].mean() < 0.075
            outer = (n[0] < 0) == right
            if outer:
                island = 7 if right else 6
                result[ids] = panel(
                    template,
                    island,
                    1 - (p[:, 1] - 4.22) / 0.55 if right else (p[:, 1] - 4.22) / 0.55,
                    (0.968 - p[:, 2]) / 0.576,
                )
            else:
                span = (p[:, 1] - 4.22) / 0.55
                result[ids] = panel(
                    template,
                    20,
                    span if right else 1 - span,
                    (0.968 - p[:, 2]) / 0.576,
                    bounds=(930, 894, 1171, 1147) if right else (683, 894, 925, 1147),
                )
        else:
            for upper in (True, False):
                selected = sel & ((normals[:, 2] > 0) == upper)
                if selected.sum() < 20:
                    continue
                ids, chord = base.profile_coordinates(
                    pos, loops, triangles, faces, selected, 0, 1
                )
                p = pos[loops[ids]]
                span = (p[:, 0] + 0.53) / 1.21
                flap = drs or p[:, 2].mean() > 0.915
                island = 5 if flap and not upper else 10 if upper else 15
                result[ids] = panel(
                    template,
                    island,
                    1 - span if island == 5 else span,
                    chord if upper else 1 - chord,
                    bounds=((673, 505, 1176, 583) if flap else (673, 583, 1176, 741))
                    if island == 10
                    else None,
                )
    return result


def author(name, pos, loops, uv, triangles, faces, starts, normals, template):
    args = (pos, loops, uv, triangles, faces, starts, normals, template)
    if name == "body_object":
        return body(*args)
    if name == "front_wing_mixer":
        return front(*args)
    if name.startswith("rear_wing_no_mod"):
        return rear(*args, name.endswith("_drs"))
    result = np.tile(CARBON, (len(loops), 1))
    p = pos[loops]
    if "wheel_cover_" in name:
        centre = (pos.min(0) + pos.max(0)) / 2
        result = np.column_stack(
            (
                1141 + (p[:, 1] - centre[1]) / 0.55 * 204,
                1568 - (p[:, 2] - centre[2]) / 0.55 * 204,
            )
        )
    elif name == "mirror_main_body_at01":
        for left, island in [(False, 13), (True, 16)]:
            ids = np.flatnonzero((p[:, 0] > 0.0598) == left)
            span = (p[ids, 1] - 1.42) / 0.25
            result[ids] = panel(
                template, island, span if left else 1 - span, (0.77 - p[ids, 2]) / 0.14
            )
    elif name == "floor_rb_mixer":
        centres = np.array(
            [
                pos[loops[a:b]].mean(0)
                for a, b in zip(starts, np.r_[starts[1:], len(loops)])
            ]
        )
        edge = (
            (np.abs(centres[:, 0] - 0.0598) > 0.65)
            & (centres[:, 1] > 1.3)
            & (centres[:, 1] < 3.8)
        )
        edge &= (centres[:, 2] > 0.06) & (centres[:, 2] < 0.18)
        for left, island in [(False, 24), (True, 35)]:
            ids = np.flatnonzero((edge & ((centres[:, 0] > 0.0598) == left))[faces])
            fraction = (p[ids, 1] - 1.5) / 0.9
            result[ids] = panel(
                template,
                island,
                fraction,
                (0.18 - p[ids, 2]) / 0.12 if left else (p[ids, 2] - 0.06) / 0.12,
            )
    return result
