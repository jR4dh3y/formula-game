#!/usr/bin/env bash
# The downloaded blend is data. Its embedded Python must never execute.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
blender=${BLENDER:-/usr/sbin/blender}
python=${PYTHON:-python3}
source=${GEN22_SOURCE:-$repo/raw/gen22/fp04rb_new_rig.blend}
output=${1:-${ASSET_DIR:-$repo/assets}/cars/gen22}
textures=${GEN22_TEXTURES:-$repo/raw/gen22/source-textures}
work=${GEN22_WORK:-$repo/raw/gen22/pbr}
template_year=${GEN22_TEMPLATE_YEAR:-2025}
if [[ "$template_year" == 2025 ]]; then
    "$python" -I "$repo/tools/livery/template_islands.py" \
        --template "$repo/raw/gen22/mapping-reference/2025-template.png" \
        --out "$repo/raw/gen22/livery-mapping/2025" --dark-wireframe --thresh 220
fi
"$python" -I "$repo/tools/car/prepare_textures.py" "$textures" "$work/textures"
"$blender" -b --factory-startup --disable-autoexec "$source" --python-exit-code 1 --python "$repo/tools/car/prepare_gen22.py" -- "$output" --textures "$work/textures" --work "$work" --template-year "$template_year"
"$blender" -b --disable-autoexec --factory-startup --python-exit-code 1 --python "$repo/tools/car/check_gen22.py" -- "$output"
