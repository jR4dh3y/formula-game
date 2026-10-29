"""Road-mesh rasterization and main-loop extraction, in metres (Y up)."""

from __future__ import annotations

import itertools

import networkx as nx
import numpy as np
from PIL import Image, ImageDraw
from scipy.ndimage import binary_closing, distance_transform_edt, gaussian_filter1d
from skimage.morphology import remove_small_holes, skeletonize


def road_grid(triangles, resolution=0.5, close=True):
    flat = triangles[:, :, [0, 2]]
    origin = np.floor(flat.min(axis=(0, 1))) - 10
    size = np.ceil((flat.max(axis=(0, 1)) - origin + 10) / resolution).astype(int)
    if int(size[0]) * int(size[1]) > 64_000_000:
        raise ValueError(
            "road raster exceeds 64 million cells; use tiled/bridge-aware conversion for this track"
        )
    canvas = Image.new("1", tuple(size), 0)
    draw = ImageDraw.Draw(canvas)
    for t in flat:
        draw.polygon([tuple(p) for p in (t - origin) / resolution], fill=1)
    mask = np.asarray(canvas)
    if close:
        mask = binary_closing(mask, iterations=2)
        mask = remove_small_holes(mask, max_size=int(100 / resolution**2))
    return mask, origin


def sample_height(points, triangles):
    """Barycentric road elevation, including sloped/banked triangle interiors."""
    flat = triangles[:, :, [0, 2]]
    lo = flat.min(axis=1) - 0.75
    hi = flat.max(axis=1) + 0.75
    result = []
    for point in points:
        ids = np.flatnonzero(((lo <= point) & (hi >= point)).all(axis=1))
        if not len(ids):
            raise ValueError(f"centerline outside road at {point}")
        t = flat[ids]
        a = t[:, 0]
        b = t[:, 1] - a
        c = t[:, 2] - a
        d = point - a
        det = b[:, 0] * c[:, 1] - b[:, 1] * c[:, 0]
        safe = np.where(np.abs(det) > 1e-9, det, 1)
        u = (d[:, 0] * c[:, 1] - d[:, 1] * c[:, 0]) / safe
        v = (b[:, 0] * d[:, 1] - b[:, 1] * d[:, 0]) / safe
        weights = np.column_stack([1 - u - v, u, v])
        inside = (weights >= 0).all(axis=1)
        projected = (t * weights[:, :, None]).sum(axis=1)
        distances = np.linalg.norm(projected - point, axis=1)
        distances[~inside] = np.inf
        for edge in range(3):
            nxt = (edge + 1) % 3
            start = t[:, edge]
            delta = t[:, nxt] - start
            fraction = ((point - start) * delta).sum(axis=1) / np.maximum(
                (delta * delta).sum(axis=1), 1e-12
            )
            fraction = fraction.clip(0, 1)
            q = start + delta * fraction[:, None]
            d = np.linalg.norm(q - point, axis=1)
            better = d < distances
            weights[better] = 0
            weights[better, edge] = 1 - fraction[better]
            weights[better, nxt] = fraction[better]
            distances[better] = d[better]
        distances[np.abs(det) < 1e-9] = np.inf
        best = np.argmin(distances)
        if result:
            candidates = np.flatnonzero(
                distances <= max(0.1, float(distances[best]) + 0.01)
            )
            candidates_y = (triangles[ids[candidates], :, 1] * weights[candidates]).sum(
                axis=1
            )
            best = candidates[np.argmin(np.abs(candidates_y - result[-1]))]
        # Raster boundaries are quantized to 0.5 m. Permit only subpixel
        # source cracks; a larger missing driving surface is a conversion error.
        if distances[best] > 0.75:
            raise ValueError(f"centerline outside road at {point}")
        y = triangles[ids[best], :, 1]
        result.append(float(y @ weights[best]))
    return np.asarray(result)


def resample(points, spacing):
    closed = np.vstack([points, points[0]])
    s = np.r_[0, np.cumsum(np.linalg.norm(np.diff(closed, axis=0), axis=1))]
    out = np.arange(0, s[-1], spacing)
    return np.column_stack(
        [np.interp(out, s, closed[:, i]) for i in range(points.shape[1])]
    )


def extract(triangles, config, debug, route=None, edges=None):
    resolution = config.get("resolution", 0.5)
    mask, origin = road_grid(triangles if route is None else route, resolution)
    for polygon in config.get("exclude", []):
        canvas = Image.fromarray(mask)
        draw = ImageDraw.Draw(canvas)
        draw.polygon(
            [tuple((np.array(p) - origin) / resolution) for p in polygon], fill=0
        )
        # Pillow mode-1 arrays can store true as byte 255. Normalize boolean
        # storage before passing it to the skeletonizer C extension.
        mask = np.asarray(canvas) != 0
    skeleton = skeletonize(mask)
    coords = np.argwhere(skeleton)
    indices = {tuple(p): i for i, p in enumerate(coords)}
    graph = nx.Graph()
    for i, (y, x) in enumerate(coords):
        for dy, dx in [(0, 1), (1, -1), (1, 0), (1, 1)]:
            other = indices.get((y + dy, x + dx))
            if other is None:
                continue
            if dy and dx and ((y, x + dx) in indices or (y + dy, x) in indices):
                continue
            graph.add_edge(i, other)
    if config.get("guide"):
        from scipy.interpolate import CubicSpline

        guide = np.array(config["guide"], dtype=float)
        guide = np.vstack([guide, guide[0]])
        s = np.r_[0, np.cumsum(np.linalg.norm(np.diff(guide, axis=0), axis=1))]
        p = CubicSpline(s, guide, bc_type="periodic")(np.arange(0, s[-1], 1.0))
        p = gaussian_filter1d(p, 2, axis=0, mode="wrap")
        length = np.linalg.norm(np.roll(p, -1, axis=0) - p, axis=1).sum()
        best = (abs(length / config["length"] - 1), p, length)
    else:
        cycles = [c for c in nx.cycle_basis(graph) if len(c) * resolution > 100]
        cycles.sort(key=len, reverse=True)
        print(
            "skeleton cycles", [round(len(c) * resolution) for c in cycles], flush=True
        )
        # XOR cycles enumerate alternative connected layouts without choosing pit lanes
        # by their material name. The measured lap length selects a candidate; no scale
        # fitting is performed. Config exclusions resolve ambiguous layouts.
        edges = [{tuple(sorted(e)) for e in zip(c, c[1:] + c[:1])} for c in cycles[:12]]
        best = None
        for bits in itertools.product((False, True), repeat=len(edges)):
            chosen = set()
            for enabled, es in zip(bits, edges):
                if enabled:
                    chosen.symmetric_difference_update(es)
            if not chosen:
                continue
            g = nx.Graph()
            g.add_edges_from(chosen)
            if any(d != 2 for _, d in g.degree()) or not nx.is_connected(g):
                continue
            start = min(g)
            order = [start]
            prev = None
            current = start
            while True:
                nxt = next(n for n in g[current] if n != prev)
                if nxt == start:
                    break
                order.append(nxt)
                prev, current = current, nxt
            p = coords[order][:, ::-1] * resolution + origin
            p = resample(p, 1.0)
            p = gaussian_filter1d(p, config.get("smooth", 2.0), axis=0, mode="wrap")
            length = np.linalg.norm(np.roll(p, -1, axis=0) - p, axis=1).sum()
            score = abs(length / config["length"] - 1)
            if best is None or score < best[0]:
                best = (score, p, length)
    if best is None:
        raise ValueError("no closed racing loop; inspect road materials/exclusions")
    _, points, length = best
    print(f"chosen loop {length:.1f} m / expected {config['length']} m", flush=True)
    start = np.array(config.get("start", points[0].tolist()))
    i = np.argmin(np.linalg.norm(points - start, axis=1))
    points = np.roll(points, -i, axis=0)
    direction = np.array(config.get("direction", (points[1] - points[0]).tolist()))
    if np.dot(points[1] - points[-1], direction) < 0:
        points = points[np.r_[0, np.arange(len(points) - 1, 0, -1)]]
    points = resample(points, 3.0)
    physical, _ = road_grid(triangles, resolution, close=False)
    physical_origin = _
    for polygon in config.get('exclude',[]):
        canvas=Image.fromarray(physical)
        draw=ImageDraw.Draw(canvas)
        draw.polygon([tuple((np.array(p)-physical_origin)/resolution) for p in polygon],fill=0)
        physical=np.asarray(canvas)!=0
    physical = binary_closing(physical, iterations=config.get("gap_closing", 1))
    physical = remove_small_holes(physical, max_size=16)
    if edges is not None:
        points = refine_edges(points, edges, physical_origin, resolution, physical)
    distance = distance_transform_edt(physical) * resolution
    # A rubbered racing line or coarse route hint identifies the layout, not
    # the road centre. Move each station to the midpoint of its asphalt cross
    # section. Wide connected runoff is deliberately excluded from this step.
    from scipy.ndimage import binary_closing as close_samples

    offsets = np.arange(-20, 20.01, 0.5)
    for _ in range(4 if route is not None else 0):
        tangent = np.roll(points, -2, axis=0) - np.roll(points, 2, axis=0)
        tangent /= np.maximum(np.linalg.norm(tangent, axis=1, keepdims=True), 1e-9)
        normal = np.column_stack([-tangent[:, 1], tangent[:, 0]])
        samples = points[:, None, :] + normal[:, None, :] * offsets[None, :, None]
        q = np.rint((samples - physical_origin) / resolution).astype(int)
        q[:, :, 0] = q[:, :, 0].clip(0, physical.shape[1] - 1)
        q[:, :, 1] = q[:, :, 1].clip(0, physical.shape[0] - 1)
        inside = close_samples(
            physical[q[:, :, 1], q[:, :, 0]], structure=np.ones((1, 3))
        )
        for i, row in enumerate(inside):
            if not row[40]:
                continue
            left = 40
            right = 40
            while left > 0 and row[left - 1]:
                left -= 1
            while right < 80 and row[right + 1]:
                right += 1
            width = offsets[right] - offsets[left]
            if 7 <= width <= 25:
                shift = np.clip((offsets[right] + offsets[left]) / 2, -4, 4)
                points[i] += normal[i] * shift
        points = gaussian_filter1d(points, 1.2, axis=0, mode="wrap")
    safe = distance >= config.get("min_clearance", 3.0)
    _, nearest = distance_transform_edt(~safe, return_indices=True)
    for _ in range(8):
        pixels = np.rint((points - physical_origin) / resolution).astype(int)
        invalid = ~safe[pixels[:, 1], pixels[:, 0]]
        if not invalid.any():
            break
        q = pixels[invalid]
        points[invalid] = (
            nearest[:, q[:, 1], q[:, 0]][::-1].T * resolution + physical_origin
        )
        points = gaussian_filter1d(points, 2.0, axis=0, mode="wrap")
    pixels = np.rint((points - physical_origin) / resolution).astype(int)
    invalid = ~safe[pixels[:, 1], pixels[:, 0]]
    q = pixels[invalid]
    points[invalid] = (
        nearest[:, q[:, 1], q[:, 0]][::-1].T * resolution + physical_origin
    )
    render_debug(debug, mask, origin, resolution, points, config)
    points = resample(points, 3.0)
    pixels = np.rint((points - physical_origin) / resolution).astype(int)
    invalid = ~safe[pixels[:, 1], pixels[:, 0]]
    q = pixels[invalid]
    points[invalid] = (
        nearest[:, q[:, 1], q[:, 0]][::-1].T * resolution + physical_origin
    )
    if edges is None:
        heights = sample_height(points, triangles)
    else:
        from scipy.spatial import cKDTree

        vertices = np.unique(edges.reshape(-1, 3), axis=0)
        tree = cKDTree(vertices[:, [0, 2]])
        _, neighbors = tree.query(points, k=24)
        heights = []
        for point, indices in zip(points, neighbors):
            nearby = vertices[indices]
            matrix = np.column_stack([nearby[:, [0, 2]] - point, np.ones(len(nearby))])
            coefficients = np.linalg.lstsq(matrix, nearby[:, 1], rcond=None)[0]
            heights.append(coefficients[2])
        heights = gaussian_filter1d(heights, 1.5, mode="wrap")
    points3 = np.column_stack([points[:, 0], heights, points[:, 1]])
    pixels = np.rint((points - physical_origin) / resolution).astype(int)
    widths = distance[pixels[:, 1], pixels[:, 0]]
    widths = np.minimum(widths, config.get("max_half_width", float("inf")))
    render_debug(debug, mask, origin, resolution, points, config)
    if abs(length / config["length"] - 1) > 0.02:
        raise ValueError(f"lap length {length:.1f} outside 2% of {config['length']}")
    return points3, widths


def render_debug(path, mask, origin, resolution, points, config):
    canvas = Image.fromarray(np.uint8(mask) * 130).convert("RGB")
    draw = ImageDraw.Draw(canvas)

    def pixel(p):
        return tuple((np.array(p) - origin) / resolution)

    draw.line([pixel(p) for p in points] + [pixel(points[0])], fill="#00ffff", width=3)
    for i, p in enumerate(points[::100]):
        draw.text(pixel(p), str(i * 300), fill="#ffff00")
    p = pixel(points[0])
    draw.ellipse((p[0] - 10, p[1] - 10, p[0] + 10, p[1] + 10), fill="red")
    lo = origin
    hi = origin + np.array(mask.shape[::-1]) * resolution
    for x in np.arange(np.ceil(lo[0] / 100) * 100, hi[0], 100):
        p = pixel([x, lo[1]])
        draw.line((p[0], 0, p[0], canvas.height), fill="#333333")
        draw.text((p[0] + 2, 10), str(int(x)), fill="white")
    for z in np.arange(np.ceil(lo[1] / 100) * 100, hi[1], 100):
        p = pixel([lo[0], z])
        draw.line((0, p[1], canvas.width, p[1]), fill="#333333")
        draw.text((10, p[1] + 2), str(int(z)), fill="white")
    canvas.thumbnail((1800, 1800))
    canvas.save(path)


def refine_edges(points, triangles, origin, resolution, physical):
    """Fit a seeded route between the mesh's painted road-edge strips."""
    canvas = Image.new("1", (physical.shape[1], physical.shape[0]), 0)
    draw = ImageDraw.Draw(canvas)
    for triangle in triangles[:, :, [0, 2]]:
        draw.polygon([tuple(p) for p in (triangle - origin) / resolution], fill=1)
    edges = np.asarray(canvas) != 0
    offsets = np.arange(-35, 35.01, 0.25)
    for _ in range(10):
        tangent = np.roll(points, -3, axis=0) - np.roll(points, 3, axis=0)
        tangent /= np.maximum(np.linalg.norm(tangent, axis=1, keepdims=True), 1e-9)
        normals = np.column_stack([-tangent[:, 1], tangent[:, 0]])
        probes = points[:, None, :] + normals[:, None, :] * offsets[None, :, None]
        pixels = np.rint((probes - origin) / resolution).astype(int)
        pixels[:, :, 0] = pixels[:, :, 0].clip(0, edges.shape[1] - 1)
        pixels[:, :, 1] = pixels[:, :, 1].clip(0, edges.shape[0] - 1)
        hits = edges[pixels[:, :, 1], pixels[:, :, 0]]
        for i, row in enumerate(hits):
            indices = np.flatnonzero(row)
            if len(indices) < 2:
                continue
            groups = np.split(indices, np.flatnonzero(np.diff(indices) > 1) + 1)
            positions = [float(offsets[g].mean()) for g in groups if len(g) < 20]
            candidates = []
            for a, b in itertools.combinations(positions, 2):
                width = b - a
                if not 8 <= width <= 24:
                    continue
                centre = (a + b) / 2
                midpoint = points[i] + normals[i] * centre
                q = np.rint((midpoint - origin) / resolution).astype(int)
                if not physical[q[1], q[0]]:
                    continue
                score = abs(centre) + abs(width - 14) * 0.5
                candidates.append((score, centre))
            if candidates:
                _, shift = min(candidates)
                points[i] += normals[i] * np.clip(shift, -5, 5)
        points = gaussian_filter1d(points, 2.0, axis=0, mode="wrap")
    return points
