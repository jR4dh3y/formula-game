# F1 Racecraft

Bevy is the sole active game engine. New gameplay, rendering, input, audio and UI work belongs in `game/` and `sim/`. The previous custom engine and its original client are preserved as a frozen, independent workspace under [archive/custom-engine](archive/custom-engine/README.md); do not add features there or restore it as an active dependency.

A Bevy 0.20 racing game with a deterministic 360 Hz simcade simulation. Select a circuit, car and driving aids from the in-game menu, then race against up to 15 AI drivers. The default RB6 field matches the V8-era sound and proportions; the RB22 provides the modern car and matching field. The simulation includes manual gearing, tyre temperature and wear, wing setup, slipstream, dirty air, car contacts, track collisions, timing and results.

## Run

Install Rust 1.97.1 or newer and your platform's Bevy build dependencies. On Linux this includes a Vulkan driver, development libraries for Wayland/X11, ALSA and udev, and a linker.

```sh
cargo run -p f1-the-game -- --assets /path/to/assets --auto-gears
```

The generated assets are separate from this repository. [Download the complete assets ZIP](https://fbs.jr4.in/s/f1-assets) using this permanent link. Extract the ZIP into the repository root to create `./assets`, or point `--assets` at an existing converted asset tree:

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

Normal launches open Race setup. Choose the circuit, car, laps, AI field, grid slot, tyres and wings, then select Start race. The race waits for its scenes to load before starting the countdown. `--quick-race` skips the menu using the CLI configuration; `--frames` also skips it for capture/testing. Run `cargo run -p f1-the-game -- --help` for the full CLI.

The menu discovers converted circuits in the selected asset directory. The original asset tree on this machine contains **Spa, Silverstone, Bahrain and COTA**, plus **RB6 and RB22**. Deferred Nürburgring assets remain disabled until their drivable-surface conversion is repaired. To add a circuit, provide `tracks/<id>/{track.json,collision.bin,visual.glb}`. The worktree's ignored `assets` symlink points at the original asset tree for convenient local launching.

The Graphics tab offers Low, Medium, High (default) and Ultra presets, applied immediately and saved with your preferences; `--graphics low|medium|high|ultra` sets one from the command line. Leaving the window during a race opens the pause menu, and rendering is throttled while the window is unfocused.

Driving aids has Beginner, Balanced and Unassisted presets. Beginner is the default, including automatic gears. Steering guidance follows the racing line when you release steering; strong manual steering overrides it. Corner braking anticipates bends, adds brake pressure and reduces a held accelerator. ABS, traction and stability are separate controls. You still control acceleration. The coloured racing line previews corner speed, and STEER / BRAKE indicators light up during interventions. Pause-menu changes apply immediately. Unassisted disables all aids and automatic shifting.

Aids and per-car camera positions persist in `$XDG_CONFIG_HOME/f1-racecraft/preferences.json` (or `~/.config/f1-racecraft/preferences.json`). `F1_SETTINGS` overrides the file. Explicit CLI aid settings override saved aids; `--quick-race` and capture runs use their CLI configuration without reading or writing preferences. Start new race replaces the current session; Resume race keeps it.

## Performance

On Linux with the NVIDIA driver under Wayland, the game presents through XWayland: NVIDIA's native Wayland presentation waits for each frame to finish before returning, which serialized CPU and GPU work (set `F1_WAYLAND=1` to keep native Wayland). Sky lighting is generated once per circuit rather than every frame. Together these took the High preset from 115 to 236 fps uncapped on an RTX 3050 laptop with unchanged output. Vertical sync is on by default; turn it off in Graphics or with `--no-vsync`.

- `--benchmark DIR` runs the comparison workload: two computer-controlled cars
  already moving, separated on the racing line, for 20 seconds. It checks the
  complete drive before opening the window and rejects contact or recovery.
  The measured drive must match that preflight exactly. Loading and warm-up
  are excluded, and VSync is disabled.
- Use the same `--benchmark-seed N` (default 24301), `--benchmark-start M`,
  track, car, camera, resolution and graphics preset for comparisons. Six
  seeded random sample times cover the run. Their exact states are rendered
  after timing, so PNG readback does not inflate frame times.
- Each fresh output directory contains `benchmark.json`, `frames.csv` and six
  PNGs. The report includes frame-time percentiles, 1% low FPS, workload state
  hashes and asset fingerprints. Changed asset fingerprints are expected when
  comparing visual changes; keep them equal when measuring code alone.
- `--frames N --capture FILE` remains a capture utility. Its old FPS log is
  diagnostic, not the repeatable moving benchmark.
- `F1_DETERMINISTIC=1` with a capture run advances exactly 1/60 s per frame and restarts temporal jitter with the race, so captures from different builds can be compared pixel by pixel.
- `F1_PROFILE=1` logs per-pass CPU and GPU timings; `--features profile` writes a Chrome trace for ui.perfetto.dev.
- F3 shows frame rate and frame times in game.

The **Benchmark** tab in the main or pause menu launches this same workload in
a separate window, preserving your current race. It uses the selected circuit,
car, camera, graphics preset and current window resolution. When it finishes,
the tab shows average FPS, 1% low FPS, p95 frame time and a selectable output
folder under `$XDG_CACHE_HOME/f1-racecraft/benchmarks` (or
`~/.cache/f1-racecraft/benchmarks`). Sound is muted during the test.

```sh
cargo run --release -p f1-the-game -- --assets ./assets --track spa --car rb6 \
  --camera chase --size 1920x1080 --graphics high --mute \
  --benchmark "$HOME/.cache/racecraft-baseline" --benchmark-seed 24301
```

`tools/verify_benchmark.sh ASSETS NEW_OUTPUT_DIRECTORY` runs two identical
native Vulkan benchmarks on an isolated Weston display and verifies the
reports, screenshot files and repeated drive hashes. It requires Weston and
a Vulkan-capable GPU. Frame times can differ with GPU load and clock speed;
the repeatability check compares the workload, not a performance threshold.

`tools/verify_settings.sh ASSETS NEW_OUTPUT_DIRECTORY` checks all six menu
tabs, pause and camera-editor layouts at 1920×1080, 958×1078 and 640×600. It
clicks the Benchmark button on the wide layout, waits for the real child
process and report, and checks that the original race stays paused and intact.

## Portable Linux build

`CARGO_BUILD_JOBS=3 python3 tools/package/build_appimage.py` builds a single
`dist/F1-Racecraft-0.1.0-linux-x86_64.AppImage` containing the game, both cars,
all four playable tracks, sound, fonts and runtime libraries. Send that file;
players do not need Rust, the repository, separate assets or FUSE installed.
They need an x86-64 Linux desktop (kernel 6.1+) and a working Vulkan GPU driver. The image
automatically extracts and runs when mounting is unavailable. See
[packaging notes](tools/package/README.md) for controls, rebuilding and provenance.

## Controls

| Action | Keyboard | Gamepad |
| --- | --- | --- |
| Throttle / brake | W / S or up / down | RT / LT |
| Steer | A / D or left / right | Left stick |
| Shift up / down | E or Shift / Q or Ctrl | RB or A / LB or X |
| Cycle camera | C | Y |
| Adjust camera against live scene | F4; F4 / Esc to resume | — |
| Cockpit / TV pod / third person | 1 / 2 / 3, or on-screen buttons | Cycle with Y |
| Recover to track | R | Back / Select |
| Pause menu / resume | Escape or P | Start |
| Mute / unmute | M | — |
| Frame-rate readout | F3 | — |
| Quit | Quit game in menu, or window close | Window close |

There are exactly three camera modes: seated cockpit, TV pod above the driver's head, and third-person chase. Use 1 / 2 / 3, C, or Cameras & controls. F4 pauses the race with a live camera preview: adjust height, forward position, pitch, field of view, vibration and horizon stabilization. Positions save separately for RB6 and RB22. Camera cuts reset temporal history; chase smoothing accounts for car velocity to avoid trailing farther behind at speed. `--manual-gears` disables automatic shifting. Keyboard steering and brake pressure are smoothed; gamepad steering has a deadzone and response curve.

The RB6 and RB22 cockpit steering wheels turn with the interpolated front-wheel steering angle, including steering assistance, at a 6:1 ratio. The existing wheel geometry and its attached controls rotate around the wheel centre; the dashboard stays fixed. This uses the steering materials in the supplied converted car models.

The HUD includes the timing tower, lap times, countdown lights, race messages, shift lights, speed, pedals, tyre temperatures/wear, lock-up/sliding indicators, aero effects, minimap and results.

Open **HUD & map** from Race setup or the pause menu to choose between **Digital cockpit**, with illuminated segments and a neutral gear aperture, and **Analog telemetry**, with separate metal-rimmed RPM, speed, pedal and G-force instruments. Tyre temperatures and live tow/wake effects use needle gauges in both layouts. The unboxed minimap supports Full circuit and Nearby track modes. Driving-aid buttons, camera shortcut strips and map headings are omitted while racing; their settings and keyboard shortcuts remain available. The G-force plot uses one G per ring. Both layouts use live simulation data and leave the physical cockpit wheel visible.

The minimap setting is independent: **Full circuit** shows the whole field; **Nearby track** follows the car through a 600 m window with the road ahead pointing up. Both choices apply immediately, survive new races, and are saved with your other preferences. Automated capture runs use the defaults and never write your preferences.

## Engine sound

At the user's request, the player engine samples the engine audio directly from [their F1 2013 RB9 video reference](https://www.youtube.com/watch?v=P19YW7daEFM&t=286s). Separate acceleration and overrun excerpts retain the recorded engine texture. The samples are pitch-stabilized to a common 18,000 RPM and phase-aligned, then continuously resampled against the simulation's RPM. Throttle blends load, upshifts briefly cut ignition, and downshifts blend a rev-match blip. Pause and mute fade smoothly. Press M to toggle sound, or launch with `--mute`.

The samples are embedded from `game/audio/reference`, independently of `--assets`. See [source and processing credits](game/audio/reference/CREDITS.md). The earlier F60 harmonic voice and F430 loop pack are retained under their separate [CC BY-SA credits](game/audio/v8/CREDITS_LICENSE.md), but are no longer played.

`python3 tools/audio/prepare_engine.py /path/to/reference-240-305.mp4` rebuilds the sampled loops from the video's 4:00–5:05 section (requires NumPy/SciPy and ffmpeg). Export a 14-second listening sample with `cargo test -p f1-the-game export_engine_demo -- --ignored --nocapture`; it writes `/tmp/f1-smooth-engine.wav`.

Track and car PNG textures receive linear-light mipmaps and 16× anisotropic filtering at load time. Temporal antialiasing stabilizes fine detail; camera changes reset its history to avoid carrying the old view into the new one.

Imported car paint is calibrated as coated composite rather than polished metal; tyres stay matte. Car contact shadows, short-range ambient occlusion, higher-resolution shadow maps and additional daylight fill help the cars sit on the track. Chase and onboard views use tighter framing.

## Verify

```sh
cargo test --workspace --lib --bins
# Both HUDs and both map modes, menu radio input, wide/tall captures and saved preferences:
XDG_RUNTIME_DIR=/run/user/$(id -u) tools/verify_hud.sh
F1_TEST_ASSETS=/path/to/assets cargo test -p f1-the-game spa_guidance -- --ignored --nocapture
F1_TEST_ASSETS=/path/to/assets cargo test -p racecraft-sim --test spa -- --nocapture
cargo run -p f1-the-game -- --assets /path/to/assets --frames 180 --capture /tmp/f1-bevy.png
# Full renderer check with injected controls, camera editor and twelve screenshots:
F1_TEST_ASSETS=/path/to/assets cargo test -p f1-the-game rendered_game_flows -- --ignored --nocapture
# Real menu pointer input, live aids, four circuit switches and both car models:
F1_TEST_ASSETS=/path/to/full/original/assets cargo test -p f1-the-game rendered_menu_and_track_selection -- --ignored --nocapture
F1_TEST_ASSETS=/path/to/assets cargo test -p f1-the-game --test cli_exit -- --ignored --nocapture

# Audio device check (plays a brief engine sound):
cargo test -p f1-the-game live_engine_playback -- --ignored --nocapture
```

Capture counts frames after scene loading and exits after writing the image. Missing assets and screenshot-write errors return a failing exit code. Spa integration tests print a skip notice when no generated assets are available; use `F1_TEST_ASSETS` to run them against an external tree.

The ignored renderer tests need a Vulkan-capable display. The gameplay check verifies scene hierarchy, car materials, driving, all three cameras, pause/resume and recovery, and writes grid/cockpit/onboard/paused/racing/results screenshots plus both instrument layouts and both map modes under `/tmp/f1-bevy-proof`. Its results screenshot uses a forced finish state; race completion is covered by the simulation tests. The menu check clicks actual egui tabs, aid presets, both HUD/map selectors and Start, switches through all four circuits and both cars, checks that old scene roots are removed, and confirms live aid changes while paused. Set `F1_TEST_OUTPUT` to choose a capture directory. `tools/verify_hud.sh` runs these checks on an isolated Weston/XWayland display and captures landscape and tall-window layouts; it requires Weston and a Vulkan-capable GPU. The existing Spa AI lap-time benchmark still fails its unchanged target; see MIGRATION.md for the baseline comparison.

The CLI exit test launches separate game processes, quits after 1 and 12 frames, captures the first frame, and checks a failed screenshot write. Pipeline compilation runs on Bevy's render thread to prevent early shutdown racing a background GPU compiler worker. First-use pipeline compilation can block a frame; this avoids the reproduced NVIDIA teardown crash.

## Code

- `game/`: Bevy app, glTF entities, input, three camera modes, daylight, menus and HUD through `bevy_egui`.
- `sim/`: renderer-independent physics, AI and race rules. Post-migration polish fixes reverse engine braking, repeated shifts during a torque cut, and stale crash dynamics after recovery while preserving tyre condition and timing.
- `tools/`: asset converters.
- `archive/custom-engine/`: complete frozen custom-engine workspace, excluded from the active build. Includes the legacy engine, client, simulation, lockfile and asset tools from the last custom-engine `main` revision.

See [MIGRATION.md](MIGRATION.md) for migration coverage and recorded verification limits. The historical gameplay design in [the archived handoff](archive/custom-engine/handoff.md) also describes future features; those are not all implemented.
