# Bevy migration

Exit condition: the existing race runs through Bevy, with no custom-engine dependency in the game, and the real Spa/car assets render in a captured frame.

The migration started on `t3/recreate-game-bevy` at `8da559c`. Bevy is now the only active game engine; the latest custom-engine workspace is frozen at `archive/custom-engine/`, with source tag `archive/custom-engine-2026-10-29` pointing to `d0679e2`. The Bevy branch incorporates main's tyre-aware braking updates and multi-track asset tools. The verification notes below retain historical results from earlier migration stages.

## Constraints

- Preserve unrelated changes. Stage only task files. Do not commit or push.
- Use the existing simulation and asset contracts as the source of gameplay behavior.
- Test at the running renderer boundary; compilation alone does not prove rendering.
- Keep credentials and private transcripts out of output and source.
- No host desktop, GPU binding, or VM configuration changes.
- No broad casts, speculative compatibility layer, or dead custom-engine calls in the game.
- Do not claim one working path proves the whole change. Record applicable paths and untested limits separately.
- Do not stop at a plan or partial patch; complete implementation and available validation. PR-specific instructions do not apply because no PR or Git publication was requested.

## Ownership

`racecraft-sim` owns the deterministic 360 Hz race, AI, collisions, assists, powertrain, tyres, timing and results. Bevy owns scheduling, window lifecycle, glTF scenes, PBR materials, lights, transforms, input and camera. `bevy_egui` draws the existing HUD using Bevy's renderer. No renderer or ECS compatibility layer is retained. The old engine and client are preserved in the independent, excluded `archive/custom-engine/` workspace; there is no active root `engine/` crate.

## Coverage and completion ledger

Post-migration client additions: race/pause menus with automatic asset discovery, four available circuits and two cars, individual live driving-aid controls, and exactly three selectable cameras (cockpit, TV pod, third person). Deferred Nürburgring conversion remains unavailable. The simulation polish agent fixed reverse engine braking, gear skipping during active shifts and stale recovery state; regression tests cover all three. The pre-existing Spa lap-time target remains unchanged and failing.

The subsequent driving/presentation pass adds actual racing-line steering guidance and anticipatory corner braking, independently of ABS. Beginner defaults include automatic gears; manual steering overrides guidance. A one-minute real-asset Spa regression with only throttle held covers 2.9 km, reaches seventh gear and records zero off-track ticks; the same input without guidance goes off track for 18,942 of 21,600 ticks. The HUD shows applied pedals, intervention indicators and corner target speed. Camera framing now has per-car defaults, velocity-aware chase smoothing, horizon stabilization and an F4 live-preview editor. Aids and per-car camera settings persist in user preferences; automated runs do not touch those files. Unit checks now pass 51 tests, and strict workspace Clippy passes. Renderer checks cover both cars, four map switches, live aid presets, moving onboard views and the paused camera editor. The original AI lap-time ceiling is not relaxed.

| Surface | Applies | Check | State |
| --- | --- | --- | --- |
| CLI: track, assets, setup, grid, field size, laps, assists, gears, window | Yes | Options tests, 8/16-car launches, invalid arguments and missing track | proved |
| Track and RB22/RB6 glTF scenes, PBR, daylight, shadows | Yes | RTX 3050 Vulkan runtime; grid, racing and full-grid PNGs inspected | proved |
| Car body pose, front steering, four spinning wheels | Yes | Loaded hierarchy counts and transform tests | proved |
| Fixed simulation, AI, contacts, race timing, finish | Yes | 27 sim unit tests; fixed-schedule integration; Spa baseline comparison below | proved |
| Keyboard, analog gamepad, queued shifts, disconnect | Yes | Input tests and renderer test with injected Bevy keyboard state | proved |
| Four cameras, cycle back to chase, recovery | Yes | Renderer assertions on all modes, moving car recovery | proved |
| HUD tower, timing, lights, messages, tyres, aero, minimap, results | Yes | Four renderer captures inspected; finish state forced for results presentation | proved |
| Pause/resume, Escape quit | Yes | Fixed-schedule and actual renderer assertions | proved |
| Frame-limited capture and asset-load errors | Yes | CLI PNG, 1/12-frame exit, first-frame capture, failed capture write and missing-asset exits | proved |
| Multiplayer, remote/relay, mobile | No | No implementation in original game client | N/A |
| Player engine audio | Added after migration | Direct user-reference acceleration/overrun samples; continuous RPM resampling, shifts, limiter, pause/mute | See README audio section |
| Custom-engine API and renderer tests | No | The game no longer consumes those APIs; engine source is archived | N/A |
| Docs and active workspace dependencies | Yes | README, cargo metadata/tree, no Dhool dependency in game/lockfile | proved |

Generated assets are available at `/home/radhey/code/f1-the-game/assets` on this host. Use `--assets` to test them without copying generated files into this worktree.

## Verification on 2026-10-29

- `cargo build -p f1-the-game`: passed.
- `cargo check --workspace --tests`: passed.
- `cargo test --workspace --lib --bins`: 34 passed, 3 opt-in tests ignored. Two ignored tests are the simulation's existing debug probes; the third is the renderer test run separately below.
- `cargo clippy -p f1-the-game --all-targets --no-deps -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `F1_TEST_ASSETS=/home/radhey/code/f1-the-game/assets cargo test -p f1-the-game rendered_game_flows -- --ignored --nocapture`: passed on NVIDIA RTX 3050 Vulkan, using a private Weston Wayland display. All eight car scenes and 32 wheel nodes loaded. Driving, all camera modes, pause/resume, recovery, Escape quit and four PNG writes passed. Grid, pause, racing and results images were inspected. Race messages are cleared when showing final results so they do not bleed through the panel.
- Normal CLI capture with 8 cars and maximum 16-car grid: passed. Help returned 0; invalid dimensions, missing assets/track and capture without a frame limit returned 1.
- `cargo test -p f1-the-game --test cli_exit -- --ignored --nocapture` with the display and asset environment set: passed. Separate game processes exited normally after 1 and 12 frames, saved a first-frame PNG, and returned exit code 1 for a screenshot write to a missing directory.

An early frames-only exit initially reproduced SIGSEGV on an Async Compute worker while the main thread was tearing down NVIDIA graphics resources. GDB confirmed the crash. Bevy's `RenderPlugin::synchronous_pipeline_compilation` removes that background pipeline-worker lifetime from every exit path, including normal keyboard/window exits. The setting is applied to all runs, not just capture mode. The regression above passes with it enabled. First-use compilation may block a frame; no upstream driver or Bevy internals were modified.

## Existing failures and limits

The real-asset Spa integration test fails its unchanged best-lap requirement of less than 150 seconds. All six AI drivers finish; the best lap is `157.46666666660988` seconds, top speed is 322 km/h, with 46 contacts. The original `8da559c` simulation **and original Cargo.lock with glam 0.33.1** were copied into a temporary test workspace and run against the same generated assets. Every entrant's lap time, off-track distance, contact count and top speed matched the migrated workspace exactly. The Bevy migration does not change the simulation or relax this test. `F1_TEST_ASSETS=... cargo test --workspace` therefore still fails this baseline assertion.

Strict Clippy including path dependencies also finds the original `unnecessary_lazy_evaluations` warning at `sim/src/track/collision.rs:243`. The changed game client passes strict Clippy independently. Physics source is unchanged.

No physical gamepad was connected. Gamepad tests inject analog values, button presses and disconnection into Bevy's input types. Physical keyboard/gamepad play, manual window-close interaction, other operating systems and long-session performance were not tested. The renderer ran on a private compositor, not the user's physical display. Results presentation was forced in the render test; actual race completion is covered by the simulation tests, including the Spa run that fails only its lap-time ceiling.

Captures and logs are preserved under `target/bevy-migration-proof/` as ignored local artifacts. The Rust test and CLI commands above reproduce the checks. The one-time build took about 22 minutes with the existing optimized dependency profile and three build jobs; later builds were incremental.

The historical starting revision above is not the current Git state. The Bevy migration, gameplay/presentation improvements, portable packaging and custom-engine archive are committed for the transition to `main`. No remote is configured, so this transition is a local merge.

## Final engine transition

The root workspace contains only `game` and `sim`, with no Dhool dependency.
`archive/custom-engine/` preserves all 97 tracked files from `d0679e2`
byte-for-byte, plus an archive README. Its own Cargo workspace and lockfile
remain independent. `AGENTS.md` and the Cursor project rule direct future
development to Bevy and keep the archive frozen.

After merging main's simulation and asset-tool changes: 52 Rust unit tests,
three real-asset track tests (both cars on Bahrain, COTA and Silverstone), the
one-minute Spa driving-guidance regression, six asset-tool tests, strict
workspace Clippy, formatting and whitespace checks pass. The Spa field test
still fails the existing 150-second lap ceiling: all six entrants finish,
best lap 151.73 seconds. That threshold remains unchanged. The archived
workspace was checked through Cargo metadata and source-integrity comparison;
its optional legacy build is not part of the active checks.
