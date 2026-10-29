#!/usr/bin/env bash
# Exercise every menu tab and launch the real benchmark through its UI button.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if [[ ${1:-} == --inside ]]; then
    output=$2 size=$3 executable=$4 game=$5 launch=$6
    status=0
    env -u WAYLAND_DISPLAY -u F1_WAYLAND -u F1_TEST_BENCHMARK_LAUNCH \
        WGPU_BACKEND=vulkan F1_TEST_SIZE="$size" F1_TEST_OUTPUT="$output" \
        F1_TEST_GAME_BINARY="$game" \
        ${launch:+F1_TEST_BENCHMARK_LAUNCH=1} \
        "$executable" --exact menu::proof::rendered_responsive_settings --ignored --nocapture \
        > "$output/runtime.log" 2>&1 || status=$?
    printf '%s\n' "$status" > "$output/exit-status"
    exit "$status"
fi
if [[ $# != 2 ]]; then
    echo "usage: $0 ASSET_DIRECTORY NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
export F1_TEST_ASSETS=$(realpath -- "$1")
output=$(realpath -m -- "$2")
mkdir -- "$output"
cd "$repo"
cargo build --release -p f1-the-game
probe=$(cargo test -p f1-the-game --bin f1-the-game --no-run --message-format=json |
    python3 -c 'import json,sys; rows=[json.loads(line) for line in sys.stdin]; print(next(r["executable"] for r in rows if r.get("reason")=="compiler-artifact" and r.get("profile",{}).get("test") and r.get("executable")))')
cp --reflink=auto -- "$probe" "$output/.settings-probe"
cp --reflink=auto -- target/release/f1-the-game "$output/.game-bin"
trap 'rm -f -- "$output/.settings-probe" "$output/.game-bin"' EXIT
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
for case in landscape:1920x1080:launch portrait:958x1078: compact:640x600:; do
    IFS=: read -r name size launch <<< "$case"
    mkdir "$output/$name"
    weston --backend=headless --renderer=gl --xwayland --socket="racecraft-settings-$$-$name" \
        --width="${size%x*}" --height="${size#*x}" --idle-time=0 --no-config -- \
        "$repo/tools/verify_settings.sh" --inside "$output/$name" "$size" \
        "$output/.settings-probe" "$output/.game-bin" "$launch" \
        > "$output/$name/weston.log" 2>&1
    [[ -f "$output/$name/exit-status" && $(< "$output/$name/exit-status") == 0 ]]
    echo "$name: menu interactions and viewport bounds passed"
done
printf 'Settings proof: %s\n' "$output"
