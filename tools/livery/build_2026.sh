#!/usr/bin/env bash
# Build the Monoposto 2026 -> FP04RB livery mapping from scratch.
#
# Usage: tools/livery/build_2026.sh
#
# Overrides (environment):
#   LIVERY_WORK    work directory (default: <repo>/raw/gen22/livery-mapping)
#   PYTHON         Python interpreter (default: python3)
#   TEMPLATE_2026  2026 template PNG
#                  (default: <repo>/raw/gen22/mapping-reference/2026-template.png)
#
# Needs the six FP04RB parts-mixer OBJs in $LIVERY_WORK/obj/.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
WORK=${LIVERY_WORK:-$REPO/raw/gen22/livery-mapping}
PY=${PYTHON:-python3}
TEMPLATE=${TEMPLATE_2026:-$REPO/raw/gen22/mapping-reference/2026-template.png}

OBJ_PARTS=(main_body front_wing rear_wing mirrors floor cockpit_foam)
missing=()
for part in "${OBJ_PARTS[@]}"; do
    [[ -f "$WORK/obj/fp04rb_$part.obj" ]] || missing+=("fp04rb_$part.obj")
done
if (( ${#missing[@]} > 0 )); then
    echo "error: missing FP04RB OBJ files in $WORK/obj/: ${missing[*]}" >&2
    echo "Copy ~/Downloads/fp04rb/3D_parts_mixer/*.obj into $WORK/obj/ and rerun." >&2
    exit 1
fi
if [[ ! -f "$TEMPLATE" ]]; then
    echo "error: 2026 template not found: $TEMPLATE (set TEMPLATE_2026 to override)" >&2
    exit 1
fi

echo "[1/7] template islands -> $WORK"
"$PY" -I "$HERE/template_islands.py" --template "$TEMPLATE" --out "$WORK"

echo "[2/7] bake FP04RB texel maps -> $WORK/bake2k"
"$PY" -I "$HERE/bake_fp04rb.py" "$WORK/obj" "$WORK/bake2k" 2048

echo "[3/7] unroll main body -> $WORK/bake2k/main_body_s.npz"
"$PY" -I "$HERE/unroll_body.py" "$WORK/obj/fp04rb_main_body.obj" "$WORK/bake2k/main_body_s.npz" 2048

echo "[4/7] base map (wings, endplates, halo) -> $WORK/map2026.npz"
"$PY" -I "$HERE/build_map_2026.py" "$WORK/bake2k" "$WORK/islands.npy" "$WORK/map2026.npz"

echo "[5/7] body patches -> $HERE/patches_2026.json"
"$PY" -I "$HERE/make_patches_2026.py" --work "$WORK" --out "$HERE/patches_2026.json"

echo "[6/7] harmonic body map -> $WORK/map_v3.npz"
"$PY" -I "$HERE/build_map_v3.py" --spec "$HERE/patches_2026.json" --base "$WORK/map2026.npz" \
    --bake "$WORK/bake2k/main_body.npz" --islands "$WORK/islands.npy" --out "$WORK/map_v3.npz"
"$PY" -I "$HERE/refine_mesh.py" --obj "$WORK/obj/fp04rb_main_body.obj" \
    --bake "$WORK/bake2k/main_body.npz" --spec "$HERE/patches_2026.json" \
    --islands "$WORK/islands.npy" --map "$WORK/map_v3.npz"
"$PY" -I "$HERE/make_patches_2026.py" --work "$WORK" --merge-halo "$WORK/map_v3.npz"

echo "[7/7] checks"
"$PY" -I "$HERE/check_stretch.py" "$WORK/map_v3.npz" "$WORK/bake2k"
"$PY" -I "$HERE/check_handedness.py" "$WORK/map_v3.npz" "$WORK/bake2k"
"$PY" -I "$HERE/audit_mapping.py" "$WORK/map_v3.npz" "$WORK/bake2k" --out "$WORK/map_v3.audit.json"
echo "done: $WORK/map_v3.npz"
