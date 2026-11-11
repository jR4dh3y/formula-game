#!/usr/bin/env bash
# Build the 2026 team cars from their Sketchfab GLBs (see gen26_teams.json).
#
#   tools/car/build_gen26.sh <source dir> [asset dir]
#
# <source dir> holds one GLB per team, named <team>.glb (e.g. ferrari.glb), as
# downloaded from each model's Sketchfab page at 4096 px textures. Output goes
# to <asset dir>/cars/gen26/teams/<team>/, and the base team's car becomes
# <asset dir>/cars/gen26/car.{glb,json} and car_lod1.glb.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
source_dir=${1:?usage: build_gen26.sh <source dir> [asset dir]}
assets=${2:-$repo/assets}
blender=${BLENDER:-blender}
python=${PYTHON:-python3}
teams=$("$python" -c 'import json,sys; print(" ".join(json.load(open(sys.argv[1]))["teams"]))' \
    "$repo/tools/car/gen26_teams.json")
base=$("$python" -c 'import json,sys; print(json.load(open(sys.argv[1]))["base"])' \
    "$repo/tools/car/gen26_teams.json")
for team in $teams; do
    model="$source_dir/$team.glb"
    if [[ ! -f $model ]]; then
        echo "skipping $team: $model not found" >&2
        continue
    fi
    "$blender" -b --disable-autoexec --factory-startup -P "$repo/tools/car/prepare_gen26.py" -- \
        "$model" "$assets/cars/gen26/teams/$team"
done
for name in car.glb car_lod1.glb car.json; do
    cp "$assets/cars/gen26/teams/$base/$name" "$assets/cars/gen26/$name"
done
