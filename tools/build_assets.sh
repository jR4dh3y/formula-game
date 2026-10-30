#!/usr/bin/env bash
# Regenerate local assets from data archives; never execute archive contents.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
zip_dir=${ZIP_DIR:-$repo}
output=${ASSET_DIR:-$repo/assets}
python=${PYTHON:-python3}
blender=${BLENDER:-blender}
selection=${1:-all}
export OPENBLAS_NUM_THREADS=${OPENBLAS_NUM_THREADS:-1}
temporary=()
cleanup() { for directory in "${temporary[@]}"; do rm -rf -- "$directory"; done; }
trap cleanup EXIT
extract() {
    extracted=$("$python" "$repo/tools/track/extract_zip.py" "$zip_dir/$1")
    temporary+=("$extracted")
}
if [[ $selection == all || $selection == spa ]]; then
    if [[ -n ${SPA_SOURCE:-} ]]; then
        spa=$SPA_SOURCE
    else
        spa=$(mktemp -d /tmp/racecraft-spa-XXXXXX)
        temporary+=("$spa")
        git clone --depth 1 --branch v0.9.2 https://codeberg.org/leillo1975/ardennen-spa "$spa"
    fi
    "$python" "$repo/tools/track/convert_spa.py" "$spa" "$output/tracks/spa"
fi
if [[ $selection == all || $selection == cars ]]; then
    extract '2010-redbull-rb6.zip'
    "$blender" -b --disable-autoexec --factory-startup -P "$repo/tools/car/prepare_car.py" -- \
        "$extracted/source/redbull rb6.glb" "$output/cars/rb6" \
        --wheel-materials wheel,wheel.001,tread,tyre --max-texture 1024
    "$repo/tools/car/build_gen22.sh" "$output/cars/gen22"
fi
found=false
for config in "$repo"/tools/track/tracks/*.json; do
    name=$(basename "$config" .json)
    if [[ $selection != all && $selection != tracks && $selection != "$name" ]]; then continue; fi
    found=true
    archive=$("$python" -c 'import json,sys; print(json.load(open(sys.argv[1]))["archive"])' "$config")
    model=$("$python" -c 'import json,sys; print(json.load(open(sys.argv[1]))["model"])' "$config")
    extract "$archive"
    "$python" "$repo/tools/track/convert_glb.py" "$config" "$extracted/$model" "$output/tracks/$name"
done
if [[ $found == false && $selection != all && $selection != spa && $selection != cars ]]; then
    echo "unknown asset selection: $selection (use all, spa, cars, tracks, or a track directory name)" >&2
    exit 2
fi
