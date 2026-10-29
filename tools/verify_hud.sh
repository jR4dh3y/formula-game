#!/usr/bin/env bash
# Run from any directory; isolate renderer input from the live desktop's focus.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export F1_TEST_ASSETS="${F1_TEST_ASSETS:-$repo_root/assets}"
proof_output="${F1_TEST_OUTPUT:-$(mktemp -d /tmp/f1-hud-proof.XXXXXX)}"
mkdir -p "$proof_output"

cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

render_case() {
    local case_name="$1" size="$2" test_name="$3"
    local case_output="$proof_output/$case_name"
    local case_status="$case_output/status-$$"
    mkdir -p "$case_output"
    weston --backend=headless --renderer=gl --xwayland \
        --socket="f1-hud-$$-$case_name" --idle-time=0 \
        --width="${size%x*}" --height="${size#*x}" --no-config \
        --log="$case_output/weston.log" -- \
        env -u WAYLAND_DISPLAY -u F1_WAYLAND \
        F1_TEST_OUTPUT="$case_output" F1_TEST_SIZE="$size" \
        bash -c '
            status=0
            cargo test -p f1-the-game "$2" -- --ignored --nocapture || status=$?
            printf "%s\n" "$status" > "$1"
            exit "$status"
        ' _ "$case_status" "$test_name"
    # Weston itself can exit successfully after its child test fails.
    if [[ ! -f "$case_status" ]] || [[ "$(< "$case_status")" != 0 ]]; then
        printf 'Renderer test failed: %s\n' "$test_name" >&2
        return 1
    fi
}

render_case landscape 1920x1080 rendered_game_flows
render_case portrait 958x1078 rendered_game_flows
render_case menus 1280x720 rendered_menu_and_track_selection
printf 'HUD proof captures: %s\n' "$proof_output"
