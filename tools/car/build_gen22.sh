#!/usr/bin/env bash
# The downloaded blend is data. Its embedded Python must never execute.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
blender=${BLENDER:-blender}
python=${PYTHON:-python3}
source=${GEN22_SOURCE:-$repo/raw/gen22/fp04rb_new_rig.blend}
output=${1:-${ASSET_DIR:-$repo/assets}/cars/gen22}
bake=${GEN22_BAKE:-$repo/raw/gen22/bake}
overrides=${GEN22_OVERRIDES:-$repo/raw/gen22/liveries}
"$blender" -b --disable-autoexec "$source" --python-exit-code 1 --python "$repo/tools/car/prepare_gen22.py" -- "$output" --bake "$bake"
"$python" -I "$repo/tools/car/livery.py" --bake "$bake" --output "$output" --overrides "$overrides"
"$blender" -b --disable-autoexec --factory-startup --python-exit-code 1 --python "$repo/tools/car/check_gen22.py" -- "$output"
