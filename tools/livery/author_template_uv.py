"""Author Monoposto UV1 on the source mesh, without resampling a team sheet.

Run in Blender with --disable-autoexec. Original UVs identify source seams only;
each wing surface is flattened independently on the actual mesh graph.
"""
from pathlib import Path
import json
import sys

import numpy as np
from scipy.sparse import coo_matrix, diags
from scipy.sparse.csgraph import connected_components
from scipy.sparse.linalg import spsolve
from scipy.spatial import cKDTree

REPO = Path(__file__).resolve().parents[2]
PAINT = {
    'body_object', 'front_wing_mixer', 'rear_wing_no_mod',
    'rear_wing_no_mod._drs', 'mirror_main_body_at01',
    'floor_rb_mixer', 'cockpit_inner', 'body_top_wing', 'body_side_top_wings',
}
# A plain carbon swatch shared by the supplied sheets, outside printed panels.
CARBON = np.array([25.0, 1550.0])


def source_arrays(obj):
    """Read base topology before subdivision, in the source's world frame."""
    mesh = obj.data
    mesh.calc_loop_triangles()
    pos = np.array([obj.matrix_world @ v.co for v in mesh.vertices])
    loops = np.array([l.vertex_index for l in mesh.loops])
    uv = np.array([l.uv[:] for l in mesh.uv_layers[0].data])
    triangles = np.array([t.loops[:] for t in mesh.loop_triangles])
    faces = np.repeat(np.arange(len(mesh.polygons)), [p.loop_total for p in mesh.polygons])
    starts = np.array([p.loop_start for p in mesh.polygons])
    return pos, loops, uv, triangles, faces, starts


def components(triangles, count):
    a, b = triangles.ravel(), np.roll(triangles, 1, axis=1).ravel()
    graph = coo_matrix((np.ones(len(a)), (a, b)), shape=(count, count)).tocsr()
    return connected_components(graph, directed=False)[1]


def charts(loops, uv, triangles, starts):
    # Weld only loops sharing both a source vertex and a UV. Overlapping charts
    # at different vertices must never become one surface.
    _, keys = np.unique(np.column_stack((loops, np.round(uv, 5))), axis=0, return_inverse=True)
    labels = components(keys[triangles], keys.max() + 1)
    return labels[keys[starts]]


def rectangle(pos, triangles, guide):
    """Positive-weight harmonic disk with arc-length boundary constraints.

    Source UVs choose four corners and their winding. They do not supply the
    interior coordinates. A convex boundary prevents flipped interior faces.
    """
    edges = np.sort(np.concatenate((triangles[:, [0, 1]], triangles[:, [1, 2]],
                                    triangles[:, [2, 0]])), axis=1)
    unique, counts = np.unique(edges, axis=0, return_counts=True)
    boundary = unique[counts == 1]
    neighbours = {}
    for a, b in boundary:
        neighbours.setdefault(a, []).append(b)
        neighbours.setdefault(b, []).append(a)
    if not neighbours or any(len(v) != 2 for v in neighbours.values()):
        raise ValueError('surface must have a closed, manifold boundary')
    start = min(neighbours)
    ring, previous, current = [start], start, neighbours[start][0]
    while current != start:
        ring.append(current)
        previous, current = current, next(v for v in neighbours[current] if v != previous)
    if len(ring) != len(neighbours):
        raise ValueError('surface has more than one boundary')
    ring = np.array(ring)
    g = guide[ring]
    g = (g - g.min(0)) / np.maximum(np.ptp(g, axis=0), 1e-9)
    targets = np.array([[0, 0], [1, 0], [1, 1], [0, 1]])
    corners = [int(np.argmin(np.linalg.norm(g - target, axis=1))) for target in targets]
    # Traverse from the lower-left corner towards lower-right.
    ring = np.roll(ring, -corners[0])
    # Re-find corners after rotation; their source UV orientation is stable.
    g = guide[ring]
    g = (g - g.min(0)) / np.maximum(np.ptp(g, axis=0), 1e-9)
    corners = [int(np.argmin(np.linalg.norm(g - target, axis=1))) for target in targets]
    if corners[1] > corners[3]:
        ring = np.r_[ring[:1], ring[:0:-1]]
        corners = [0] + [len(ring) - c for c in corners[1:]]
    if not (0 < corners[1] < corners[2] < corners[3] < len(ring)):
        raise ValueError(f'could not order the four surface corners: {corners}, {len(ring)}')
    result = np.zeros((len(pos), 2))
    for i in range(4):
        ids = np.r_[ring[corners[i]:corners[i+1]+1]] if i < 3 else np.r_[ring[corners[3]:], ring[0]]
        distance = np.r_[0., np.cumsum(np.linalg.norm(np.diff(pos[ids], axis=0), axis=1))]
        f = distance / distance[-1]
        result[ids] = targets[i] + f[:, None] * (targets[(i+1) % 4] - targets[i])
    weights = 1 / np.maximum(np.linalg.norm(pos[unique[:, 0]] - pos[unique[:, 1]], axis=1), 1e-9)
    a = np.r_[unique[:, 0], unique[:, 1]]
    b = np.r_[unique[:, 1], unique[:, 0]]
    graph = coo_matrix((np.r_[weights, weights], (a, b)), shape=(len(pos), len(pos))).tocsr()
    laplacian = diags(np.asarray(graph.sum(1)).ravel()) - graph
    interior = np.setdiff1d(np.arange(len(pos)), ring)
    if len(interior):
        result[interior] = spsolve(laplacian[interior][:, interior], -laplacian[interior][:, ring] @ result[ring])
    return result


class Template:
    def __init__(self, year='2026'):
        self.year = str(year)
        directory = REPO / 'raw/gen22/livery-mapping'
        self.labels = np.load(directory / ('2025/islands.npy' if self.year == '2025' else 'islands.npy'))

    def strip(self, island, span, chord, vertical=False):
        mask = self.labels == island
        if vertical:
            mask = mask.T
        columns = np.flatnonzero(mask.any(0))
        low = np.argmax(mask[:, columns], axis=0)
        high = len(mask) - 1 - np.argmax(mask[::-1, columns], axis=0)
        x = columns[0] + .75 + np.clip(span, 0, 1) * (np.ptp(columns) - 1.5)
        a = np.interp(x, columns, low) + .75
        b = np.interp(x, columns, high) - .75
        y = a + np.clip(chord, 0, 1) * (b - a)
        return np.column_stack((y, x) if vertical else (x, y))


def flatten(pos, loops, uv, triangles, loop_faces, selection):
    tri = triangles[selection[loop_faces[triangles[:, 0]]]]
    vertices, inverse = np.unique(loops[tri], return_inverse=True)
    local_tri = inverse.reshape(-1, 3)
    guide = np.zeros((len(pos), 2))
    selected_loops = np.flatnonzero(selection[loop_faces])
    guide[loops[selected_loops]] = uv[selected_loops]
    result = rectangle(pos[vertices], local_tri, guide[vertices])
    coordinates = np.zeros((len(pos), 2))
    coordinates[vertices] = result
    return selected_loops, coordinates[loops[selected_loops]]


def profile_coordinates(pos, loops, triangles, loop_faces, selection, span_axis, chord_axis):
    """Unroll each surface's own cross-sections, never a multi-flap envelope."""
    tri = triangles[selection[loop_faces[triangles[:, 0]]]]
    edges = np.sort(np.concatenate([loops[tri[:, [0, 1]]], loops[tri[:, [1, 2]]],
                                    loops[tri[:, [2, 0]]]]), axis=1)
    edges, counts = np.unique(edges, axis=0, return_counts=True)
    ends = pos[edges[counts == 1]]
    lo, hi = ends[:, :, span_axis].min(), ends[:, :, span_axis].max()
    stations = np.linspace(lo+1e-6, hi-1e-6, 401)
    low, high = [], []
    for x in stations:
        a, b = ends[:, 0], ends[:, 1]
        dx = b[:, span_axis] - a[:, span_axis]
        use = (x >= np.minimum(a[:, span_axis], b[:, span_axis])) & (x <= np.maximum(a[:, span_axis], b[:, span_axis])) & (np.abs(dx) > 1e-10)
        f = (x-a[use, span_axis])/dx[use]
        crossings = a[use, chord_axis] + f*(b[use, chord_axis]-a[use, chord_axis])
        low.append(crossings.min() if len(crossings) else np.nan)
        high.append(crossings.max() if len(crossings) else np.nan)
    ids = np.flatnonzero(selection[loop_faces])
    p = pos[loops[ids]]
    low, high = np.array(low), np.array(high)
    good = np.isfinite(low) & np.isfinite(high)
    a = np.interp(p[:, span_axis], stations[good], low[good])
    b = np.interp(p[:, span_axis], stations[good], high[good])
    fraction = np.clip((p[:, chord_axis]-a)/np.maximum(b-a, 1e-7), 0, 1)
    return ids, fraction


def front_wing(pos, loops, uv, triangles, loop_faces, starts, normals, template):
    labels = charts(loops, uv, triangles, starts)
    physical = components(loops[triangles], len(pos))
    face_component = physical[loops[starts]]
    result = np.tile(CARBON, (len(loops), 1))
    for label in np.unique(labels):
        selection = labels == label
        centre_normal = np.median(normals[selection], axis=0)
        pts = pos[loops[np.isin(loop_faces, np.flatnonzero(selection))]]
        # Endplates use a planar chart sized in physical metres, keeping text
        # proportions. Excess 2022 endplate area samples the plain panel border.
        if abs(centre_normal[0]) > .75:
            ids = np.flatnonzero(selection[loop_faces])
            p = pos[loops[ids]]
            right = p[:, 0].mean() < .05
            outer = (centre_normal[0] < 0) == right
            island = (15 if right else 5) if outer else (12 if right else 11)
            rows, cols = np.nonzero(template.labels == island)
            # A 0.40m x 0.16m panel within the taller FP04RB endplate.
            fx = (.015 - p[:, 1]) / .40
            fy = (.30 - p[:, 2]) / .16
            upright = right == outer
            if not upright:
                fy = 1 - fy
            result[ids] = template.strip(island, fx, fy)
            continue
        component = int(np.bincount(face_component[selection]).argmax())
        component_pts = pos[physical == component]
        height = component_pts[:, 2].max()
        if height > .4:  # lower main plane, endplates and supports
            continue
        if height > .22 and centre_normal[2] < .9:
            continue
        selection &= normals[:, 2] > .15
        if selection.sum() < 100:
            continue
        # Three disconnected physical elements. Their source UVs overlap, but
        # their new UVs cannot cross a slot because they are solved separately.
        island = 34 if height > .28 else 36 if height > .22 else 37
        ids, chord = profile_coordinates(pos, loops, triangles, loop_faces, selection, 0, 1)
        p = pos[loops[ids]]
        span = ((p[:, 0] - .0458) + 1) / 2
        # The next flap overlaps the rear of this surface. Put the island on
        # the exposed chord; the covered portion stays at its plain rear edge.
        if island in (36, 37):
            chord = np.clip(chord / (.62 if island == 36 else .58), 0, 1)
        # Image-right is car-left. Image-down runs towards the leading edge,
        # whereas the surface chord above runs towards the trailing edge.
        result[ids] = template.strip(island, span, 1-chord)
    return result


def rear_wing(pos, loops, uv, triangles, loop_faces, starts, normals, template, drs):
    result = np.tile(CARBON, (len(loops), 1))
    labels = charts(loops, uv, triangles, starts)
    for label in np.unique(labels):
        selection = labels == label
        normal = np.median(normals[selection], axis=0)
        ids = np.flatnonzero(selection[loop_faces])
        p = pos[loops[ids]]
        if abs(normal[0]) > .6 and not drs:
            right = p[:, 0].mean() < .075
            outer = (normal[0] < 0) == right
            island = (41 if right else 35) if outer else (30 if right else 20)
            fx = (p[:, 1]-p[:, 1].min()) / np.ptp(p[:, 1])
            fy = (p[:, 2]-p[:, 2].min()) / np.ptp(p[:, 2])
            result[ids] = template.strip(island, 1-fy, 1-fx if right == outer else fx, vertical=True)
        else:
            # The forward and rear faces are real geometric surfaces, split at
            # their normal reversal, including the separately rigged DRS flap.
            for upper in (True, False):
                selected = selection & ((normals[:, 2] > 0) == upper)
                if selected.sum() < 50:
                    continue
                ids, chord = profile_coordinates(pos, loops, triangles, loop_faces, selected, 0, 1)
                p = pos[loops[ids]]
                span = (p[:, 0] + .53) / 1.21
                is_flap = drs or p[:, 2].mean() > .915
                if is_flap and upper:
                    # 44 and 45 are separated only by a template gutter.
                    for island, a, b in ((44, 0., .5), (45, .5, 1.)):
                        take = (chord >= a) & (chord <= b)
                        result[ids[take]] = template.strip(island, span[take], (chord[take]-a)/(b-a), vertical=True)
                else:
                    island = (46 if is_flap else 47) if not upper else 43
                    result[ids] = template.strip(island, span, chord if upper else 1-chord, vertical=True)
    return result


class PanelMesh:
    """Body seam landmarks on the source mesh, with positive graph weights."""
    def __init__(self, pos, loops, uv, triangles):
        self.P = np.column_stack((-pos[:, 1], pos[:, 2], pos[:, 0]-.0598))
        self.N = len(pos)
        self.paint = np.ones(self.N, bool)
        self.main_nodes = np.arange(self.N)
        self.kd = cKDTree(self.P)
        self.uv_tree = cKDTree(uv * [2048, -2048] + [0, 2048])
        self.loops = loops
        t = loops[triangles]
        edges = np.unique(np.sort(np.concatenate((t[:, [0, 1]], t[:, [1, 2]], t[:, [2, 0]])), axis=1), axis=0)
        length = np.linalg.norm(self.P[edges[:, 0]]-self.P[edges[:, 1]], axis=1)
        a, b = np.r_[edges[:, 0], edges[:, 1]], np.r_[edges[:, 1], edges[:, 0]]
        self.G_len = coo_matrix((np.r_[length, length], (a, b)), shape=(self.N, self.N)).tocsr()
        self.A = self.G_len.copy()
        self.A.data[:] = 1
        self.G_harm = coo_matrix((1/np.maximum(np.r_[length, length], 1e-8), (a, b)), shape=(self.N, self.N)).tocsr()

    def node_from_uv(self, uv):
        return int(self.loops[self.uv_tree.query(uv)[1]])

    def node_from_xyz(self, xyz):
        return int(self.kd.query(xyz)[1])


def body(pos, loops, uv, triangles, loop_faces, starts, normals, template):
    # Reuse the recorded anatomical seam landmarks, not the old raster solve.
    # Every coordinate is re-solved on base mesh vertices, and each face owns
    # one template island even where the source UV0 charts overlap.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import build_map_v3 as solver
    mesh = PanelMesh(pos, loops, uv, triangles)
    spec = json.loads(Path(__file__).with_name('patches_2026.json').read_text())
    patches = solver.expand_patches(spec, template.labels)
    result = np.tile(CARBON, (len(loops), 1))
    owners = np.full(len(pos), -1)
    coordinates = []
    for i, patch in enumerate(patches):
        if patch['name'].startswith('nose_cockpit'):
            continue
        shape = solver.IslandShape(template.labels, patch['island'])
        solved = solver.solve_patch(mesh, patch, shape, 2048)
        xy, _ = shape.clamp(solved['xy'])
        coords = np.full((len(pos), 2), np.nan)
        coords[solved['pn']] = xy
        coordinates.append(coords)
        owners[solved['pn']] = i
        print('BODY PANEL', patch['name'], len(solved['pn']), flush=True)
    labels = charts(loops, uv, triangles, starts)
    panel_vertices = [np.flatnonzero(np.isfinite(c[:, 0])) for c in coordinates]
    panel_trees = [cKDTree(pos[v]) for v in panel_vertices]
    face_panels = np.full(len(starts), -1)
    for face in range(len(starts)):
        ids = np.flatnonzero(loop_faces == face)
        face_owners = owners[loops[ids]]
        valid = face_owners[face_owners >= 0]
        if len(valid):
            panel = np.bincount(valid).argmax()
        elif labels[face] in (0, 1, 2, 7, 8):
            # Unowned crest/airbox faces used to interpolate from a distant
            # carbon swatch, dragging triangles across unrelated template art.
            centre = pos[loops[ids]].mean(0)
            choices = (0, 2) if centre[0] < .0598 else (1, 3)
            panel = min(choices, key=lambda i: panel_trees[i].query(centre)[0])
        else:
            continue
        face_panels[face] = panel
        coords = coordinates[panel]
        vertices = panel_vertices[panel]
        missing = ~np.isfinite(coords[loops[ids], 0])
        result[ids[~missing]] = coords[loops[ids[~missing]]]
        if missing.any():
            nearest = panel_trees[panel].query(pos[loops[ids[missing]]])[1]
            result[ids[missing]] = coords[vertices[nearest]]
    # Nose and cockpit rails have separate charts in the source. Match their
    # longitudinal stations directly, avoiding the former sheared texel fill.
    p = pos[loops]
    x, y, z = p.T
    centre = .0598
    main = np.isin(labels, [0, 1, 2])[loop_faces]
    face_centres = np.array([pos[loops[a:b]].mean(0) for a, b in
                            zip(starts, np.r_[starts[1:], len(loops)])])
    fx, fy, fz = face_centres.T
    # A polygon owns one chart. Per-loop region predicates produced triangles
    # spanning the gutters between engine, cockpit and chassis islands.
    front = main & (fy < 1.43)[loop_faces]
    rows = np.arange(2048)
    mask = template.labels == 26
    low = np.argmax(mask, axis=1)
    high = 2047-np.argmax(mask[:, ::-1], axis=1)
    yy = np.interp(y[front], [-.708, 1.43], [2040, 1100])
    width = np.interp(y[front], [-.708, -.3, .3, .8, 1.43], [.085, .105, .125, .145, .28])
    fraction = np.clip((x[front]-centre)/width, -1, 1)
    xx = 1024 + fraction * (np.interp(yy, rows, high)-np.interp(yy, rows, low))/2
    result[front] = np.column_stack((xx, yy))
    rails = main & ((fy >= 1.43) & (fy < 2.32) & (fz > .65))[loop_faces]
    yy = np.interp(y[rails], [1.43, 2.32], [1092, 660])
    left = x[rails] > centre
    xx = np.where(left, np.interp(yy, rows, high)-18, np.interp(yy, rows, low)+18)
    result[rails] = np.column_stack((xx, yy))
    # Cockpit chassis sides have their own artwork, 27/28, outside the rails.
    chassis = main & ((fy > .85) & (fy < 1.65) & (fz < .65) & (np.abs(fx-centre) > .13))[loop_faces]
    for left, island in ((False, 28), (True, 27)):
        sel = chassis & ((x > centre) == left)
        if sel.any():
            result[sel] = template.strip(island, 1-(y[sel]-.85)/.8,
                                         (.65-z[sel])/.4, vertical=True)
    # Both cover halves share longitudinal stations at the spine. An angular
    # cross-section keeps the crest continuous instead of snapping unowned
    # ridge vertices to arbitrary nearest points on the opposite side.
    cover = np.isin(face_panels, [0, 1]) & (fy >= 2.32)
    cover |= np.isin(labels, [7, 8])
    ids = np.flatnonzero(cover[loop_faces])
    yy = np.interp(y[ids], [2.32, 4.20], [638, 26])
    shoulder = np.interp(y[ids], [2.32, 2.85, 3.95, 4.20], [.82, .50, .29, .29])
    angle = np.arctan2(np.abs(x[ids]-centre), np.maximum(z[ids]-shoulder, 0)) / (np.pi/2)
    edge = np.interp(yy, rows, low)
    side = np.where(x[ids] < centre, -1, 1)
    result[ids] = np.column_stack((1023.5 + side*angle*(1021.5-edge), yy))
    # Halo ribbon, cut along its underside. The small front support is carbon.
    halo = np.isin(labels, [3, 4, 5])[loop_faces]
    ids = np.flatnonzero(halo)
    hp = p[ids]
    phi = np.arctan2((hp[:, 0]-centre)*2, 2.31-hp[:, 1])
    q = np.hypot((hp[:, 0]-centre)*2, 2.31-hp[:, 1])
    bins = np.linspace(-np.pi/2, np.pi/2, 81)
    station = np.clip(np.digitize(phi, bins)-1, 0, len(bins)-2)
    qc, zc = [], []
    for k in range(len(bins)-1):
        take = station == k
        qc.append((q[take].min()+q[take].max())/2 if take.any() else np.nan)
        zc.append((hp[take, 2].min()+hp[take, 2].max())/2 if take.any() else np.nan)
    for v in (qc, zc):
        good = np.flatnonzero(np.isfinite(v))
        v[:] = np.interp(np.arange(len(v)), good, np.array(v)[good])
    theta = np.arctan2(q-np.array(qc)[station], hp[:, 2]-np.array(zc)[station])
    fraction = .5+theta/(2*np.pi)
    ty, tx = np.nonzero(template.labels == 4)
    psi = np.arctan2(tx-1873, 438-ty)
    radius = np.hypot(tx-1873, 438-ty)
    centres = (bins[:-1]+bins[1:])/2
    inner, outer = [], []
    for a, b in zip(bins[:-1], bins[1:]):
        take = (psi >= a) & (psi < b)
        inner.append(radius[take].min() if take.any() else np.nan)
        outer.append(radius[take].max() if take.any() else np.nan)
    for v in (inner, outer):
        good = np.flatnonzero(np.isfinite(v))
        v[:] = np.interp(np.arange(len(v)), good, np.array(v)[good])
    r0, r1 = (np.interp(phi, centres, v) for v in (inner, outer))
    radius = r0+fraction*(r1-r0)
    result[ids] = np.column_stack((1873+radius*np.sin(phi), 438-radius*np.cos(phi)))
    return result


def author(obj, template):
    pos, loops, uv, triangles, loop_faces, starts = source_arrays(obj)
    transform = obj.matrix_world.to_3x3().inverted().transposed()
    normals = np.array([(transform @ p.normal).normalized() for p in obj.data.polygons])
    if template.year == '2025':
        import template_2025
        pixels = template_2025.author(obj.name, pos, loops, uv, triangles, loop_faces, starts, normals, template)
    elif obj.name == 'front_wing_mixer':
        pixels = front_wing(pos, loops, uv, triangles, loop_faces, starts, normals, template)
    elif obj.name.startswith('rear_wing_no_mod'):
        pixels = rear_wing(pos, loops, uv, triangles, loop_faces, starts, normals, template, obj.name.endswith('_drs'))
    elif obj.name == 'body_object':
        pixels = body(pos, loops, uv, triangles, loop_faces, starts, normals, template)
    elif 'wheel_cover_' in obj.name:
        island = 33 if 'front' in obj.name else 39
        rows, cols = np.nonzero(template.labels == island)
        p = pos[loops]
        centre = (pos.min(0)+pos.max(0))/2
        # Cover artwork is a planar disc; spokes and the hub stay on the
        # template's own carbon, without a source-UV projection or rebake.
        pixels = np.column_stack(((cols.min()+cols.max())/2 +
                                  (p[:, 1]-centre[1])/.55*(cols.max()-cols.min()),
                                  (rows.min()+rows.max())/2 -
                                  (p[:, 2]-centre[2])/.55*(rows.max()-rows.min())))
    elif obj.name == 'mirror_main_body_at01':
        pixels = np.tile(CARBON, (len(loops), 1))
        for left, island in ((False, 2), (True, 3)):
            take = (pos[loops, 0] > .0598) == left
            p = pos[loops[take]]
            pixels[take] = template.strip(island, (p[:, 2]-.63)/.14,
                                           (p[:, 1]-1.42)/.25, vertical=True)
    else:
        pixels = np.tile(CARBON, (len(loops), 1))
    layer = obj.data.uv_layers.get('livery') or obj.data.uv_layers.new(name='livery')
    layer.data.foreach_set('uv', (pixels / [2048, -2048] + [0, 1]).astype(np.float32).ravel())
    if template.year == '2025':
        # Face ownership survives subdivision and collapse. UV interpolation
        # alone cannot keep a triangle out of neighbouring template panels.
        owner = obj.data.attributes.get('livery_2025_panel') or obj.data.attributes.new('livery_2025_panel', 'INT', 'FACE')
        xy = np.clip(np.rint(pixels).astype(int), 0, 2047)
        labels = template.labels[xy[:, 1], xy[:, 0]].copy()
        if obj.name == 'body_object':
            rails = (pixels[:, 0] >= 276) & (pixels[:, 0] <= 673) & (pixels[:, 1] >= 803) & (pixels[:, 1] <= 1076)
            labels[rails] = 18
        for face in obj.data.polygons:
            samples = labels[list(face.loop_indices)]
            samples = samples[samples > 0]
            owner.data[face.index].value = int(np.bincount(samples).argmax()) if len(samples) else 0
    obj.data.uv_layers.active_index = 0
    obj.data.uv_layers[0].active_render = True
    return len(loops)


def apply(year='2025'):
    import bpy
    template = Template(year)
    for obj in bpy.data.collections['fp04_tr_model'].all_objects:
        if obj.type == 'MESH' and (obj.name in PAINT or 'wheel_cover_' in obj.name):
            before = np.array([l.uv[:] for l in obj.data.uv_layers[0].data])
            count = author(obj, template)
            assert np.array_equal(before, np.array([l.uv[:] for l in obj.data.uv_layers[0].data]))
            print('LIVERY UV', obj.name, count, flush=True)


if __name__ == '__main__':
    apply()
