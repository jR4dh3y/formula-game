# Archived custom-engine implementation

This is a frozen source snapshot of the last custom-engine workspace on `main`,
before Bevy became the sole active engine. It preserves the Dhool engine, original
game client, matching simulation, Cargo workspace/lockfile, asset conversion
tools and historical design handoff. Generated assets and build outputs are not
copied into the archive.

Source revision: `d0679e2` (`sim: leave braking headroom for turn-in and ABS`).
Git tag: `archive/custom-engine-2026-10-29`.

The files from that revision are preserved unchanged. This README is the only
addition to the snapshot. The archive is a separate Cargo workspace and is
explicitly excluded from the active root workspace.

All ongoing development uses Bevy in the repository's root `game/` and `sim/`.
Do not make new features here, import this engine into the active client, or
bring its renderer/runtime back into the default build. Open the archive only
for historical reference or explicitly requested legacy work.

To inspect or build the old implementation independently:

```sh
cargo metadata --manifest-path archive/custom-engine/Cargo.toml --no-deps
cargo build --manifest-path archive/custom-engine/Cargo.toml -p f1-the-game --locked
cargo run --manifest-path archive/custom-engine/Cargo.toml -p f1-the-game -- \
  --assets /absolute/path/to/converted/assets
```

The old client retains its original dependencies and requirements. Building it
is optional and does not participate in Bevy checks or releases. Active asset
conversion tools remain at the repository root, including later improvements.
