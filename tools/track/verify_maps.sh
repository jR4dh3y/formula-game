#!/usr/bin/env bash
# Render a complete deterministic lap and recovery for each playable non-Spa map.
# Assets are read only. Each run gets its own compositor and output directory.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
if [[ ${1:-} == --inside ]]; then
    assets=$2 output=$3 executable=$4
    status=0
    for track in silverstone bahrain cota; do
        mkdir -p "$output/$track"
        if env -u WAYLAND_DISPLAY -u F1_WAYLAND WGPU_BACKEND=vulkan F1_DETERMINISTIC=1 \
            F1_TEST_ASSETS="$assets" F1_TEST_TRACK="$track" F1_TEST_OUTPUT="$output/$track" \
            "$executable" --exact track_probe::rendered_track_lap --ignored --nocapture \
            > "$output/$track/runtime.log" 2>&1 &&
            rg -q 'test result: ok. 1 passed' "$output/$track/runtime.log"; then
            echo "$track: passed"
        else
            echo "$track: failed; see $output/$track/runtime.log" >&2
            status=1
        fi
    done
    echo "$status" > "$output/exit-status"
    exit "$status"
fi
if [[ $# != 2 ]]; then
    echo "usage: $0 PRIVATE_ASSET_DIRECTORY NEW_PROOF_DIRECTORY" >&2
    exit 2
fi
assets=$(realpath -- "$1")
output=$(realpath -m -- "$2")
# A fresh directory prevents stale screenshots/status from masking a failed run.
mkdir -- "$output"
cd "$repo"
executable=$(cargo test --release -p f1-the-game --bin f1-the-game --no-run --message-format=json |
    python3 -c 'import json,sys; records=[json.loads(line) for line in sys.stdin]; print(next(r["executable"] for r in records if r.get("reason")=="compiler-artifact" and r.get("profile",{}).get("test") and r.get("executable")))')
# Freeze the binary: another worktree can rebuild the shared Cargo target while
# the compositor is running. Never execute a subsequently replaced test binary.
snapshot="$output/.track-probe"
cp --reflink=auto -- "$executable" "$snapshot"
trap 'rm -f -- "$snapshot"' EXIT
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
weston --backend=headless --renderer=gl --xwayland --socket="racecraft-maps-$$" \
    --width=1280 --height=720 --idle-time=0 --no-config -- \
    "$repo/tools/track/verify_maps.sh" --inside "$assets" "$output" "$snapshot" \
    > "$output/weston.log" 2>&1
# Weston can return success even when its child failed. Check the child's status.
[[ -f "$output/exit-status" && $(cat "$output/exit-status") == 0 ]]
