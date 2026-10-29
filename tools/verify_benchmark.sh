#!/usr/bin/env bash
# Verify the real moving workload twice without taking focus from a live race.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ ${1:-} == --inside ]]; then
    output=$2 executable=$3 assets=$4
    status=0
    for run in first repeat; do
        if ! env -u WAYLAND_DISPLAY -u F1_WAYLAND WGPU_BACKEND=vulkan \
            "$executable" --assets "$assets" --track spa --car rb6 \
            --camera chase --size 1920x1080 --graphics high --mute \
            --benchmark "$output/$run" --benchmark-seed 24301 \
            > "$output/$run.log" 2>&1; then
            status=1
            break
        fi
    done
    printf '%s\n' "$status" > "$output/exit-status"
    exit "$status"
fi
if [[ $# != 2 ]]; then
    echo "usage: $0 ASSET_DIRECTORY NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
assets=$(realpath -- "$1")
output=$(realpath -m -- "$2")
mkdir -- "$output"
cd "$repo"
cargo build --release -p f1-the-game
snapshot="$output/.benchmark-bin"
cp --reflink=auto -- target/release/f1-the-game "$snapshot"
trap 'rm -f -- "$snapshot"' EXIT
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
weston --backend=headless --renderer=gl --xwayland --socket="racecraft-bench-$$" \
    --width=1920 --height=1080 --idle-time=0 --no-config -- \
    "$repo/tools/verify_benchmark.sh" --inside "$output" "$snapshot" "$assets" \
    > "$output/weston.log" 2>&1
# Weston can return zero even if its child failed.
[[ -f "$output/exit-status" && $(< "$output/exit-status") == 0 ]]
python3 - "$output" <<'PY'
import csv
import json
from pathlib import Path
import struct
import sys

root = Path(sys.argv[1])
reports = []
for name in ('first', 'repeat'):
    folder = root / name
    report = json.loads((folder / 'benchmark.json').read_text())
    assert report['duration_seconds'] == 20
    assert report['physics_ticks'] == 7200
    assert report['ai_cars'] == 2 and report['human_cars'] == 0
    assert report['contacts'] == 0 and report['minimum_clearance_metres'] >= 12
    assert report['preflight_matches_measured'] is True
    assert report['vsync'] is False
    assert all(car['speed_kmh'] > 28.8 for car in report['moving_start'])
    metrics = report['metrics']
    assert 20 <= metrics['wall_seconds'] < 22
    with (folder / 'frames.csv').open() as stream:
        frames = list(csv.DictReader(stream))
    assert len(frames) == metrics['frames'] > 0
    assert abs(float(frames[-1]['seconds']) - metrics['wall_seconds']) < 1e-6
    assert len(report['samples']) == 6
    assert len({sample['tick'] for sample in report['samples']}) == 6
    for sample in report['samples']:
        png = (folder / sample['file']).read_bytes()
        assert png[:8] == b'\x89PNG\r\n\x1a\n'
        assert struct.unpack('>II', png[16:24]) == tuple(report['resolution'])
        assert len(png) > 10000
    reports.append(report)
    print(f"{name}: 20 seconds, {len(frames)} frames, six PNGs, no contact; native run passed")
for key in ('workload', 'seed', 'track', 'car', 'camera', 'graphics', 'resolution',
            'start_metres', 'moving_start', 'final_state_fnv64', 'asset_fnv64', 'samples'):
    assert reports[0][key] == reports[1][key], f'workload differs: {key}'
print('Repeat drive, sample states and asset fingerprints match.')
PY
printf 'Benchmark evidence: %s\n' "$output"
