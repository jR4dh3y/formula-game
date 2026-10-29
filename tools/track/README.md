# Build racing assets

The asset archives are data inputs. Never run scripts or executables from them.
`extract_zip.py` extracts into a fresh `/tmp/racecraft-assets-*` directory, rejects
traversal, links, duplicate files and archives exceeding 4 GiB, and does not restore
executable permissions. `build_assets.sh` removes only its own temporary directories.
Generated assets are ignored by Git. Keep each upstream archive's attribution and
licence with your original download; the build does not grant redistribution rights.

## Rebuild everything

Install the Python tools in a virtual environment, then run the build from any directory:

```sh
python3 -m venv .venv-assets
.venv-assets/bin/pip install -r tools/track/requirements.txt
ZIP_DIR=/home/radhey/code/f1-the-game \
PYTHON="$PWD/.venv-assets/bin/python" tools/build_assets.sh
```

Blender 5.2 is required for the two cars. `BLENDER` overrides its executable.
`ZIP_DIR` defaults to the repository root. `ASSET_DIR` defaults to `<repo>/assets`.
`PYTHON` defaults to `python3`. With an existing Spa v0.9.2 checkout, use:

```sh
ZIP_DIR=/home/radhey/code/f1-the-game \
SPA_SOURCE=/tmp/f1src/ardennen-spa tools/build_assets.sh all
```

Without `SPA_SOURCE`, the script clones the upstream `v0.9.2` tag from
<https://codeberg.org/leillo1975/ardennen-spa>. The checked tag for this conversion
is `4cd1486edd58de13f6c3975a27d479b658e70539`.

The optional argument is `all`, `spa`, `cars`, `tracks`, or one configured track
name: `silverstone`, `bahrain`, `cota`. For example:

```sh
ZIP_DIR=/home/radhey/code/f1-the-game tools/build_assets.sh silverstone
cargo run --release -p f1-the-game -- --list-tracks
cargo run --release -p f1-the-game -- --track silverstone
```

Launch without `--track` to choose a circuit and setup in the HUD menu. Arrow keys
or the controller d-pad navigate; Enter or South confirms. Left/Right changes a
value, and Escape or East cancels. The race runs in a fresh process because the
engine's renderer fixes its material table at startup and winit allows one event
loop per process.

## Conversion contract

`convert_glb.py CONFIG SOURCE.glb OUTPUT_DIRECTORY` reads static glTF triangle
meshes, applies scene hierarchy transforms and metre scale, and writes:

- `visual.glb`: indexed geometry split into 512 m tiles per material. Road and
  kerb geometry retain source accuracy. Scenery is decimated when needed. Embedded
  base-colour textures are capped at 1024 pixels. Cutout textures remain `MASK`.
- `collision.bin`: original drivable triangles with surface IDs documented in
  `collision.py`, plus deduplicated wall segments. Walls require vertical faces
  at least 0.4 m tall; kerb lips do not become barriers.
- `track.json`: a closed race-direction centreline sampled at about 3 m, asphalt
  half widths, named corners and lighting. Both the extracted layout and final 3D
  length must be within 2% of the reference. No scale fitting to length is used.
- `road-debug.png`: road mask, metre grid, centreline, start and distance labels.
- `debug.png`: surfaces, walls, centreline and the starting direction.
- `conversion.json`: rendered triangles, texture count/storage, collision counts,
  length and reference length. Texture storage is the uncompressed RGBA base
  level; GPU mipmaps add storage.

Ground mesh/material names start with `ground:`. The game uses this explicit
contract instead of guessing from Spa texture names. Rebuild Spa after upgrading
the loader. The converter rejects required/compressed extensions and external
image URIs; decompress or embed those inputs before adding another source.

The reader memory-maps GLB buffers and processes primitives separately, avoiding
Blender's full-scene import overhead on large tracks. NumPy/SciPy and scikit-image
provide rasterization, distance fields and skeletonization. NetworkX selects
closed road cycles; fast-simplification reduces scenery while replaying the
vertex mapping for UVs and normals. The road raster has a 64-million-cell ceiling
so a large source cannot silently exhaust host memory.

## Configure a track

Configurations in `tracks/*.json` identify the source archive and its GLB path,
material patterns for road, asphalt, kerb, grass, gravel, concrete and walls,
reference length, start position/direction, texture/triangle budgets and corners.
The input GLBs use glTF's Y-up convention; their root transforms are baked before
classification. Scale is explicit in each configuration.

Silverstone's rubbered `groove.001` mesh selects the GP route through its many
alternate layouts; cross sections of the asphalt mesh refine its centre and
widths. Bahrain's numbered materials were checked against texture sheets and
geometry. `Mtl_73` is road, `Mtl_92/93` are other asphalt, and the pit branch is
explicitly excluded from route extraction. COTA's `fillred` and line materials
are asphalt paint, not missing ground. Its baked `shadow` polygons are omitted
because the renderer draws dynamic shadows. Temporary barriers and disconnected
1 m authoring cubes on the starting grid are removed by explicit material and
component-size rules; the surrounding terrain is retained.

Nürburgring is deferred at the user's request. Its unfinished GP configuration
is kept in `deferred/` for future work and is excluded from the build and menu.
The GP prototype still needs road/collision and full-lap validation. Nordschleife
also needs tiled extraction to stay within this machine's memory budget.

## Verify generated assets

```sh
python3 -m unittest discover -s tools/track -p 'test_*.py'
cargo test -p racecraft-sim --release --test tracks -- --nocapture
cargo test -p racecraft-sim --release --test spa
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The track integration tests skip absent generated assets. With assets present,
they drive a full lap and check completion, maximum lateral excursion and time
spent off racing surfaces. Review the two debug PNGs for every circuit, then run
and inspect a GPU capture:

```sh
./target/release/f1-the-game --track silverstone --frames 300 --capture /tmp/f1-silverstone.png
```

The engine capture hides HUD overlays and saves the final requested frame.
Menu captures include the setup HUD.
