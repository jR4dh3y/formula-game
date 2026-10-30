# Single-file Windows build

Double-click `F1-Racecraft-0.1.0-windows-x86_64.exe`. It includes the game,
Spa-Francorchamps, RB6/RB22, fonts, procedural audio and dependency notices.
No installer, Rust, Python, network connection or separate assets folder is
needed to play. Requires 64-bit Windows 10/11 and a Vulkan-capable GPU with
its current driver. This is an unsigned community build.

The native launcher decompresses the bundled game into a unique folder under
`%TEMP%`, starts the game, waits for it and any menu-launched benchmark to
exit, and removes that folder. Closing the game during a benchmark lets
that benchmark finish before cleanup.
Keep at least 500 MB of temporary space free. Extraction takes a few seconds
at each launch. A forced termination or power failure can leave the temporary
folder behind; normal exit removes it. Preferences and per-launch logs stay
in `%LOCALAPPDATA%\f1-racecraft`; benchmark output stays there as well.
Existing `F1_SETTINGS` and `XDG_CACHE_HOME` overrides are respected.
The game keeps the caller's working directory, so relative capture/benchmark
paths and relative environment overrides resolve there and survive cleanup.

Arguments are forwarded to the game, including paths containing spaces:

```powershell
.\F1-Racecraft-0.1.0-windows-x86_64.exe --graphics low --size 1280x720
```

Controls: W/S accelerate/brake, A/D steer, E/Q gears, C camera, R recover,
Esc pause. Beginner aids and automatic gears are enabled by default.

## Build from source

Use Python 3.11+, Rust 1.97.1+ with `x86_64-pc-windows-gnu`, and a 64-bit
MinGW-w64 GCC toolchain. Put its `bin` directory before any old 32-bit MinGW
installation on PATH: both `gcc` and `dlltool` must support x86-64.

Download the complete assets ZIP linked in the root README and extract it
into the repository's `assets` directory, or set `F1_PACKAGE_ASSETS` to an
existing converted asset tree. Run Cargo and the packager from the root:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
rustup default stable-x86_64-pc-windows-gnu
$env:CARGO_BUILD_JOBS = '2' # Use 1 if memory is tight.
$env:CARGO_PROFILE_RELEASE_DEBUG = '0'
python tools/package/build_windows.py
```

Output: `dist/F1-Racecraft-0.1.0-windows-x86_64.exe` and `.exe.sha256`.
The version comes from the workspace manifest. Verify the published digest
with `Get-FileHash -Algorithm SHA256` before running a downloaded release.

`--skip-build` reuses the already-built game, but still builds the launcher.
`--zip` additionally produces an unpacked-folder ZIP with `Play.cmd` for
users who prefer to extract only once. `--cargo-config PATH` supplies an
optional Cargo configuration, such as a local vendored dependency source.
The launcher is an independent utility workspace under `tools/package`, so
the active game workspace remains `game` + `sim`.

The EXE is a native PE launcher followed by a zlib-compressed file stream
and a fixed footer. It contains no runtime script or external decompressor.
All packaging is done locally; the script does not publish a release.
`BUILD.json` inside the package records the source commit, target, Rust
version and SHA-256 fingerprints of the included assets. Dependency license
texts, Rust package metadata, font license and audio credits are included.
Track/car rights remain with their respective owners.

## Validate

```powershell
cargo fmt --all -- --check
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --manifest-path tools/package/windows_launcher/Cargo.toml -- --check
cargo test --locked --manifest-path tools/package/windows_launcher/Cargo.toml --target x86_64-pc-windows-gnu --target-dir target/windows-launcher
cargo clippy --locked --manifest-path tools/package/windows_launcher/Cargo.toml --target x86_64-pc-windows-gnu --target-dir target/windows-launcher -- -D warnings
```

The single-file EXE accepts `--launcher-verify` as its first argument for an
automated, noninteractive extraction/integrity/cleanup check. It does not
start the game. Its exit status is zero on success, and its per-launch log
records the temporary path and cleanup result. Cleanup failure returns a
nonzero status, including in verification mode. Also move just the EXE to a fresh
directory and launch it from an unrelated working directory, start a race,
and confirm that its temporary directory disappears after closing the game.
