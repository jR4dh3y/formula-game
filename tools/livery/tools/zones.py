# 3D-defined paint zones for the main body texture -> masks + preview
import numpy as np, json, sys
from PIL import Image
from scipy import ndimage
R = 4096
d = np.load("livery/ref/pos_main.npz"); p = d["pos"]; n = d["nrm"]
ok = np.isfinite(p[..., 0])
X = np.nan_to_num(p[..., 0]); Y = np.nan_to_num(p[..., 1]); Z = np.nan_to_num(p[..., 2])
AX = np.abs(X); nx, ny, nz = n[..., 0], n[..., 1], n[..., 2]
halo = np.load("livery/ref/halo_mask.npy")

# ---- cockpit opening (plan view half-width by Y; front is -Y)
OY = [-0.98, -0.95, -0.88, -0.80, -0.65, -0.45, -0.25, -0.17, -0.12, -0.10]
OW = [0.00, 0.10, 0.15, 0.18, 0.20, 0.22, 0.22, 0.19, 0.12, 0.00]
def w_open(y): return np.interp(y, OY, OW, left=0, right=0)
RIM = lambda y: np.interp(y, [-1.0, -0.6, -0.3, -0.12], [0.69, 0.646, 0.648, 0.672])

# plan-view raster at 5 mm: X -0.8..0.8, Y -2.0..0.6
S = 0.002; gx = np.arange(-0.8, 0.8, S); gy = np.arange(-2.0, 0.6, S)
GX, GY = np.meshgrid(gx, gy, indexing="ij")
opening = np.abs(GX) < w_open(GY)
dist_out = ndimage.distance_transform_edt(~opening) * S          # distance from the opening
# nose taper ahead of the opening: half width 0.21 -> 0.08, rounded tip at -1.52
NY = [-1.56, -1.50, -1.40, -1.20, -0.95]; NW = [0.0, 0.07, 0.10, 0.15, 0.20]
nose = (np.abs(GX) < np.interp(GY, NY, NW, left=0, right=0)) & (GY < -0.9)
BAND = 0.10
plan_white = (dist_out <= BAND) | nose
plan_dist_edge = ndimage.distance_transform_edt(plan_white) * S   # inside distance to the white edge
def lookup(grid):
    i = np.clip(((X + 0.8) / S).astype(int), 0, len(gx) - 1); j = np.clip(((Y + 2.0) / S).astype(int), 0, len(gy) - 1)
    return grid[i, j]

deck = ok & ~halo & (Z > 0.55) & (Y < -0.02) & (Y > -1.65)
in_open = ok & (AX < w_open(Y) + 0.03) & (Y > -1.30) & (Y < -0.06)
interior = in_open & ~halo & (((nz < 0.55) & (Z < 0.80)) | (Z < RIM(Y) - 0.004))
interior |= ok & ~halo & (Y > -1.20) & (Y < -0.80) & (AX < 0.22) & (ny > 0.35) & (Z < 0.82)
band = deck & lookup(plan_white) & ~interior & (nz > -0.2) & ((Y < -0.12) | (Z < 0.76))
edge_d = lookup(plan_dist_edge)
red_pin = band & (edge_d > 0.011) & (edge_d < 0.020) & (Z < 0.74)

# ---- engine cover white (side-view rule) with stepped rear edge
ZB = 0.62
step = np.floor((Z - ZB) / 0.047).clip(0, 6)
rear = 0.30 + 0.06 * (step + 1)
eng = ok & ~halo & (Y > -0.95) & (Z > ZB) & (Y < rear) & ~((nz > 0.75) & (Z < 0.70)) \
      & ~((Y < -0.10) & (AX > 0.42)) & ~((Y < -0.30) & (ny < -0.5)) & ~interior
# white pinstripes: under the lower edge, and a diagonal behind the steps
pin_low = ok & ~halo & (Y > -0.95) & (Y < 0.36) & (Z > ZB - 0.030) & (Z < ZB - 0.021) & (np.abs(nx) > 0.3)
a = np.array([0.38, ZB - 0.025]); b = np.array([0.86, 0.95]); t = b - a; L = np.hypot(*t); t = t / L
rel_y, rel_z = Y - a[0], Z - a[1]
along = rel_y * t[0] + rel_z * t[1]; perp = rel_y * t[1] - rel_z * t[0]
pin_diag = ok & ~halo & (along > 0) & (along < L) & (np.abs(perp - 0.0) < 0.0045) & (np.abs(nx) > 0.2)
white = ((band & ~red_pin) | eng) & ~interior
white_pin = (pin_low | pin_diag) & ~white

# ---- carbon: halo (grown to swallow edge slivers), lower body below the sidepod undercut
halo_c = ndimage.binary_dilation(halo, iterations=10) & ~(white | white_pin)
# pad zones into texels no surface uses (island gutters) so filtering never bleeds base red
_, (iy, ix) = ndimage.distance_transform_edt(~ok, return_indices=True)
pad = lambda m: m[iy, ix]
white, red_pin, white_pin, interior, halo_c = map(pad, (white, red_pin, white_pin, interior, halo_c))
masks = {"white": white, "red_pin": red_pin, "white_pin": white_pin, "interior": interior, "halo": halo_c}
for k, m in masks.items():
    Image.fromarray((m * 255).astype(np.uint8)).save(f"livery/ref/zone_{k}.png")
    print(k, int(m.sum()))
# ---- preview composite
RED = (216, 18, 27); WHITE = (244, 244, 242)
img = np.zeros((R, R, 3), np.uint8); img[:] = RED
tile = np.array(Image.open("livery/ref/carbon-tile.png").convert("RGB"))
carbon = np.tile(tile, (R // tile.shape[0] + 1, R // tile.shape[1] + 1, 1))[:R, :R]
old = np.array(Image.open("livery/out/ferrari_main_body_44.png").convert("RGB"))
# keep the old carbon (lower body, floor edges) where it was dark
dark = old.max(axis=2) < 70
img[dark] = old[dark]
img[halo_c] = carbon[halo_c]
img[white] = WHITE; img[white_pin] = WHITE; img[red_pin] = RED
img[interior] = (18, 18, 20)
Image.fromarray(img).save("livery/ref/zones_preview.png")
