# F1 Racecraft development

- Bevy is the only active game engine. Implement game rendering, input, UI,
  audio and cameras in `game/` using Bevy; physics and race rules live in `sim/`.
- Run Cargo from the repository root. The active workspace is `game` + `sim`.
- `archive/custom-engine/` is a frozen, independent historical workspace.
  Do not edit it, restore Dhool as an active dependency, or build it as part of
  normal checks/releases unless the user explicitly requests legacy work.
- Keep asset conversion and packaging work in the root `tools/` directory.
- Relevant checks: `cargo test --workspace --lib --bins`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo fmt --all -- --check`. Renderer tests require a Vulkan display and
  converted assets; see README.md for their commands and known test limits.
