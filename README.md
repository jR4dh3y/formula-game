# F1 Racecraft

A Bevy 0.20 racing game with a deterministic 360 Hz simcade simulation. Race at Spa against up to 15 AI drivers in the RB22 and RB6. The simulation includes manual gearing, tyre temperature and wear, wing setup, slipstream, dirty air, car contacts, track collisions, timing and results.

## Run

Install Rust 1.97.1 or newer and your platform's Bevy build dependencies. On Linux this includes a Vulkan driver, development libraries for Wayland/X11, ALSA and udev, and a linker.

```sh
cargo run -p f1-the-game -- --assets /path/to/assets --auto-gears
```

The generated assets are separate from this repository. Point `--assets` at an existing converted asset tree, or put it at `./assets`:

```text
assets/
  tracks/spa/{track.json,collision.bin,visual.glb}
  cars/rb22/{car.json,car.glb}
  cars/rb6/{car.json,car.glb}
```

`tools/track/convert_spa.py` converts the upstream Spa track; `tools/car/prepare_car.py` prepares car models through Blender. Their module docstrings describe the source formats and arguments. The game uses the glTF files directly through Bevy. Car scenes must contain `body`, `wheel_fl`, `wheel_fr`, `wheel_rl` and `wheel_rr` nodes, with wheel pivots at their centres. The simulation and assets use +Z forward, +Y up, +X left, in metres.

For the assets already installed on this machine:

```sh
cargo run -p f1-the-game -- --assets /home/radhey/code/f1-the-game/assets
```

Run `cargo run -p f1-the-game -- --help` for setup, field size, grid position, lap count, assists, window size and capture options. The race waits for its scenes to load before starting the countdown.

## Controls

| Action | Keyboard | Gamepad |
| --- | --- | --- |
| Throttle / brake | W / S or up / down | RT / LT |
| Steer | A / D or left / right | Left stick |
| Shift up / down | E or Shift / Q or Ctrl | RB or A / LB or X |
| Camera | C | Y |
| Recover to track | R | Back / Select |
| Pause / resume | P | Start |
| Quit | Escape | Window close |

Camera cycling returns to chase after far chase, T-cam and nose. `--auto-gears` enables automatic shifting. Keyboard steering and brake pressure are smoothed; gamepad steering has a deadzone and response curve.

The HUD includes the timing tower, lap times, countdown lights, race messages, shift lights, speed, pedals, steering, tyre temperatures/wear, lock-up/sliding indicators, aero effects, minimap and results.

## Verify

```sh
cargo test --workspace
F1_TEST_ASSETS=/path/to/assets cargo test -p racecraft-sim --test spa -- --nocapture
cargo run -p f1-the-game -- --assets /path/to/assets --frames 180 --capture /tmp/f1-bevy.png
# Full renderer check with injected controls and four screenshots:
F1_TEST_ASSETS=/path/to/assets cargo test -p f1-the-game rendered_game_flows -- --ignored --nocapture
F1_TEST_ASSETS=/path/to/assets cargo test -p f1-the-game --test cli_exit -- --ignored --nocapture
```

Capture counts frames after scene loading and exits after writing the image. Missing assets and screenshot-write errors return a failing exit code. Spa integration tests print a skip notice when no generated assets are available; use `F1_TEST_ASSETS` to run them against an external tree.

The ignored renderer test needs a Vulkan-capable display. It checks the loaded scene hierarchy, drives the player, cycles every camera, pauses/resumes, recovers the car, and writes grid/paused/racing/results screenshots under `/tmp/f1-bevy-proof`. Set `F1_TEST_OUTPUT` to choose a different directory. Its results screenshot uses a forced finish state; race completion is covered by the simulation tests.

The CLI exit test launches separate game processes, quits after 1 and 12 frames, captures the first frame, and checks a failed screenshot write. Pipeline compilation runs on Bevy's render thread to prevent early shutdown racing a background GPU compiler worker. First-use pipeline compilation can block a frame; this avoids the reproduced NVIDIA teardown crash.

## Code

- `game/`: Bevy app, glTF entities, input, camera, daylight and the HUD rendered through `bevy_egui`.
- `sim/`: renderer-independent physics, AI and race rules, unchanged by the engine migration.
- `tools/`: asset converters.
- `engine/`: historical custom-engine source, excluded from the active workspace and game dependencies.

See [MIGRATION.md](MIGRATION.md) for migration coverage and recorded verification limits. The gameplay design in [handoff.md](handoff.md) also describes future features; those are not all implemented.
