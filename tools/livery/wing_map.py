"""Monoposto 2026 template -> FP04RB wing correspondence, per surface chart.

The FP04RB wing UV charts are planar projections: each UV row of an airfoil
chart is one spanwise station (constant Z) and its columns run along the chord.
Every visible surface is mapped separately:

* Front wing: islands 34/36/37 follow three separately unrolled flap surfaces
  (Ferrari: hp / V-Power / Ray-Ban). Slot height jumps split the shared planar
  UV chart before mapping. Car right is image left. The lower main plane and
  converged surfaces beneath the nose stay carbon.
* Rear wing: islands 43-47 lie with the span vertical (image top = car
  right) and unwrap the profile left to right: 43 main-plane top (leading edge
  left), 44|45 the DRS flap's forward face (Ferrari hp roundels span both),
  46 the flap's rear face from its trailing edge (IBM / ETIHAD, read from
  behind), and 47, 46's mirror, for the main-plane underside (trailing edge
  left, so its copy reads upright from behind and below).
* Front endplates: outer faces 15/5, inner faces 12/11. Each inner island is
  unfolded from its outer island across the endplate top edge.
"""
import numpy as np
from scipy import ndimage

# Region name -> template island ids.
ISLANDS = {
    "fw_flap_top": (34,), "fw_flap_mid": (36,), "fw_main": (37,),
    "fw_ep_out_right": (15,), "fw_ep_out_left": (5,),
    "fw_ep_in_right": (12,), "fw_ep_in_left": (11,),
    "rw_main_top": (43,), "rw_flap_lo": (44,), "rw_flap_hi": (45,),
    "rw_flap_back": (46,), "rw_main_under": (47,),
    "ep_out_left": (35,), "ep_out_right": (41,),
    "ep_in_left": (20,), "ep_in_right": (30,),
}
FRONT_SPAN = 2.454  # FP04RB main-plane half span at the endplates


def charts(mask, pos, nrm, min_px=50000):
    """Large connected UV charts with their median normal and position."""
    lab, n = ndimage.label(mask)
    out = []
    for i in range(1, n + 1):
        sel = lab == i
        if sel.sum() >= min_px:
            out.append({"sel": sel, "n": np.median(nrm[sel], 0), "p": np.median(pos[sel], 0)})
    return out


def row_arc(sel, pos, start_high_x):
    """Arc length along each UV row from the chord end with max (or min) X.

    Returns (arc, total) per texel, NaN outside sel. Texels are ordered by UV
    column, the chart's own chord parameter, so near-vertical faces keep their
    true length.
    """
    arc = np.full(sel.shape, np.nan)
    total = np.full(sel.shape, np.nan)
    for r in np.nonzero(sel.any(1))[0]:
        c = np.nonzero(sel[r])[0]
        P = pos[r, c]
        if (P[0, 0] < P[-1, 0]) == start_high_x:
            c, P = c[::-1], P[::-1]
        s = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(P, axis=0), axis=1))])
        arc[r, c] = s
        total[r, c] = s[-1]
    return arc, total


def column_extent(mask, smooth=5):
    """Island columns with top/bottom rows, lightly median-filtered."""
    cols = np.nonzero(mask.any(0))[0]
    lo = np.argmax(mask[:, cols], 0).astype(float)
    hi = (mask.shape[0] - 1 - np.argmax(mask[::-1, cols], 0)).astype(float)
    if smooth > 1:
        lo = ndimage.median_filter(lo, smooth, mode="nearest")
        hi = ndimage.median_filter(hi, smooth, mode="nearest")
    return cols, lo, hi


def strips(masks, xn, t):
    """Continuous strip coordinates across one or more stacked islands.

    xn in [-1, 1] runs across the islands' common columns (image left to right)
    and t in [0, 1] from the top of the first island to the bottom of the last.
    Each island takes its share of t in proportion to its height in that
    column, so the gaps between islands are never sampled.
    Returns (island index, x, y).
    """
    ext = [column_extent(m) for m in masks]
    c0 = max(e[0][0] for e in ext)
    c1 = min(e[0][-1] for e in ext)
    x = (c0 + c1) / 2 + np.clip(xn, -1, 1) * ((c1 - c0) / 2 - 0.5)
    lo = [np.interp(x, e[0], e[1]) for e in ext]
    hi = [np.interp(x, e[0], e[2]) for e in ext]
    hgt = [np.maximum(b - a, 1.0) for a, b in zip(lo, hi)]
    edges = np.cumsum([np.zeros_like(x)] + hgt, 0) / sum(hgt)
    t = np.clip(t, 0, 1)
    k = np.clip((t[None] >= edges[1:-1]).sum(0), 0, len(masks) - 1) if len(masks) > 1 else np.zeros(t.shape, int)
    y = np.empty_like(t)
    for i in range(len(masks)):
        sel = k == i
        f = (t[sel] - edges[i][sel]) / (edges[i + 1][sel] - edges[i][sel])
        y[sel] = lo[i][sel] + 0.5 + f * (hi[i][sel] - lo[i][sel] - 1)
    return k, x, y


def panel(mask, xn, t):
    """Endplate panels: island rows follow height, each row's own width the chord."""
    _, y, x = strips([mask.T], 2 * t - 1, (xn + 1) / 2)
    return x, y


def span(Z, sel):
    return Z / np.percentile(np.abs(Z[sel]), 99.9)


def split_flap(sel, pos):
    """Split the rear upper chart into DRS-flap and main-plane tops.

    Per UV row the slot is the largest height step between neighbouring
    texels in the slot zone; rows without a step are main plane only.
    """
    flap = np.zeros_like(sel)
    for r in np.nonzero(sel.any(1))[0]:
        c = np.nonzero(sel[r])[0]
        X, Y = pos[r, c, 0], pos[r, c, 1]
        dy = np.abs(np.diff(Y))
        zone = (X[:-1] > -6.16) & (X[:-1] < -5.95)
        if not zone.any():
            continue
        dy = np.where(zone, dy, 0)
        i = int(dy.argmax())
        if dy[i] < 0.02:
            continue
        rear = c[: i + 1] if X[0] < X[-1] else c[i + 1:]
        flap[r, rear] = True
    return flap


def front_elements(flap, pos):
    """Separate three flap surfaces at the slots in their shared planar chart."""
    elements = [np.zeros_like(flap) for _ in range(3)]
    for r in np.nonzero(flap.any(1))[0]:
        c = np.nonzero(flap[r])[0]
        c = c[np.argsort(-pos[r, c, 0])]
        P = pos[r, c]
        jump = np.diff(P[:, 1])
        interior = (np.arange(len(jump)) > len(c) * .12) & (np.arange(len(jump)) < len(c) * .85)
        candidates = np.where(interior & (jump > .015))[0]
        if len(candidates) < 2:
            # The flaps converge under the nose; no sponsor belongs there.
            continue
        cuts = np.sort(candidates[np.argsort(jump[candidates])[-2:]]) + 1
        for i, cols in enumerate(np.split(c, cuts)):
            elements[i][r, cols] = True
    return elements


def front_wing(d, L):
    pos, nrm, mask = d["pos"], d["nrm"], d["mask"]
    Z = pos[..., 2]
    ch = charts(mask, pos, nrm)
    upper = sorted((c for c in ch if c["n"][1] > 0.5 and abs(c["n"][2]) < 0.5), key=lambda c: c["p"][1])
    assert len(upper) == 2, "expected main-plane and flap top charts"
    parts = []
    elements = front_elements(upper[1]["sel"], pos)
    for name, sel in (("fw_main", elements[0]), ("fw_flap_mid", elements[1]),
                      ("fw_flap_top", elements[2])):
        a, total = row_arc(sel, pos, start_high_x=True)
        valid = sel & (total > 1e-8)
        fraction = np.zeros_like(a)
        np.divide(a, total, out=fraction, where=valid)
        t = 1 - fraction
        if name == "fw_main":
            # The source leading element has a deep plain-carbon border.
            # Keep its central 70% on this narrower FP04RB flap.
            t = .5 + (t - .5) * .7
        _, x, y = strips([L == ISLANDS[name][0]], -Z / FRONT_SPAN, t)
        parts.append((name, valid, x, y))
    for c in ch:
        if abs(c["n"][2]) < 0.9:
            continue
        side = np.sign(c["p"][2])          # +1 = car right
        outer = np.sign(c["n"][2]) == side
        sel = c["sel"]
        X, Y = pos[..., 0], pos[..., 1]
        fy = (Y - Y[sel].min()) / np.ptp(Y[sel])   # 1 = endplate top
        fx = (X - X[sel].min()) / np.ptp(X[sel])   # 1 = front
        # The 2022 endplate is much taller than the source panel. Fit the
        # painted panel within it instead of stretching its lettering to the tip.
        fx = .5 + (fx - .5) * 1.6
        fy = .5 + (fy - .5) * 1.4 + .25
        sel = sel & (fx >= 0) & (fx <= 1) & (fy >= 0) & (fy <= 1)
        # Outer right (15) and inner left (11) are upright; outer left (5) and
        # inner right (12) are their copies flipped across the top edge.
        upright = (side > 0) == outer
        name = f"fw_ep_{'out' if outer else 'in'}_{'right' if side > 0 else 'left'}"
        island = L == ISLANDS[name][0]
        ir, ic = np.nonzero(island)
        ex = ic.min() + .5 + fx * (np.ptp(ic) - 1)
        ey = ir.min() + .5 + (1 - fy if upright else fy) * (np.ptp(ir) - 1)
        inside = island[np.clip(np.round(ey).astype(int), 0, L.shape[0] - 1),
                        np.clip(np.round(ex).astype(int), 0, L.shape[1] - 1)]
        sel &= inside
        parts.append((name, sel, ex, ey))
    return parts


def rear_wing(d, L):
    pos, nrm, mask = d["pos"], d["nrm"], d["mask"]
    Z = pos[..., 2]
    ch = charts(mask, pos, nrm)
    flat = [c for c in ch if abs(c["n"][2]) < 0.5]
    up = [c for c in flat if c["n"][1] > 0.5]
    down = sorted((c for c in flat if c["n"][1] < 0), key=lambda c: c["p"][0])
    assert len(up) == 1 and len(down) == 2, "expected upper, flap-rear and main-under charts"
    parts = []

    def add(names, sel, zn, t):
        """Span along island rows (top = car right), chord t across columns."""
        k, row, col = strips([(L == ISLANDS[n][0]).T for n in names], -zn, t)
        for i, name in enumerate(names):
            parts.append((name, sel & (k == i), col, row))

    upper = up[0]["sel"]
    flap_top = split_flap(upper, pos)
    main_top = upper & ~flap_top
    flap_back, main_under = down[0]["sel"], down[1]["sel"]
    # Main-plane top: leading edge on the left, trailing edge (actuator) right.
    a, s = row_arc(main_top, pos, start_high_x=True)
    add(("rw_main_top",), main_top, span(Z, main_top), a / s)
    # DRS flap forward face: leading edge in 44, trailing edge at the right of 45.
    a, s = row_arc(flap_top, pos, start_high_x=True)
    add(("rw_flap_lo", "rw_flap_hi"), flap_top, span(Z, flap_top), a / s)
    # DRS flap rear face: continues from 45 at the trailing edge, on the left.
    a, s = row_arc(flap_back, pos, start_high_x=False)
    add(("rw_flap_back",), flap_back, span(Z, flap_back), a / s)
    # Main-plane underside: 47 mirrors 46; seen from behind and below its
    # trailing edge is the upper edge, so it starts on the left like 46.
    a, s = row_arc(main_under, pos, start_high_x=False)
    add(("rw_main_under",), main_under, span(Z, main_under), a / s)

    X, Y, nz = pos[..., 0], pos[..., 1], nrm[..., 2]
    for c in ch:
        if abs(c["n"][2]) < 0.6:
            continue
        side = np.sign(c["p"][2])
        outer = np.sign(c["n"][2]) == side
        sel = c["sel"] & (np.abs(nz) > 0.6)
        fy = (Y - Y[sel].min()) / np.ptp(Y[sel])
        fx = (X - X[sel].min()) / np.ptp(X[sel])
        # Read from the viewing side: front on the viewer's left for the left
        # outer and right inner faces.
        front_left = outer == (side < 0)
        name = f"ep_{'out' if outer else 'in'}_{'right' if side > 0 else 'left'}"
        ex, ey = panel(L == ISLANDS[name][0], 1 - 2 * fx if front_left else 2 * fx - 1, 1 - fy)
        parts.append((name, sel, ex, ey))
    return parts


def pack(parts, mask, names, L):
    """Region/xy/fill/snap arrays, snapping stray samples into their island."""
    shape = mask.shape
    region = np.full(shape, -1, np.int8)
    xy = np.full(shape + (2,), np.nan, np.float32)
    snap = np.zeros(shape, np.float16)
    for name, sel, x, y in parts:
        if not sel.any():
            continue
        isl = np.isin(L, ISLANDS[name])
        _, idx = ndimage.distance_transform_edt(~isl, return_indices=True)
        xs, ys = x[sel], y[sel]
        r = np.clip(np.round(ys).astype(int), 0, L.shape[0] - 1)
        c = np.clip(np.round(xs).astype(int), 0, L.shape[1] - 1)
        inside = isl[r, c]
        nr, nc = idx[0][r, c], idx[1][r, c]
        xy[sel, 0] = np.where(inside, xs, nc)
        xy[sel, 1] = np.where(inside, ys, nr)
        snap[sel] = np.where(inside, 0, np.hypot(nc - xs, nr - ys))
        region[sel] = names.index(name)
    fill = np.where(mask, 2, 0).astype(np.int8)   # FILL_CARBON elsewhere
    fill[region >= 0] = 0
    return {"xy": xy, "fill": fill, "region": region, "snap": snap}
