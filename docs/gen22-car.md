# Ground-effect car (`gen22`) asset and livery contract

The game's primary car is one shared 2022-regulation ("ground effect") model,
painted per team. The 2010 RB6 moves to **Legacy cars**. The old RB22 asset is
retired once `gen22` is in place.

Source: `raw/gen22/fp04rb_new_rig.blend` (ignored, copy of the downloaded
`fp04rb_new_rig.blend`). Always open it with `blender -b --disable-autoexec`.
In the blend the car points along **-Y**, up is +Z, the driver's left is +X,
and units are metres. The rigged pose is yawed about 0.4 degrees and offset
about 6 cm in X; the converter straightens and centres it between the wheels.

## Generated files (`assets/cars/gen22/`, not tracked)

```text
car.glb          player model (full cockpit, steering wheel, driver)
car_lod1.glb     opponent model, same node and material contract
car.json         geometry and rig metadata (below)
teams.json       copy of tools/car/teams.json, read by the game
liveries/<team>/paint.png     base colour for material `paint` (2048 px, sRGB)
liveries/<team>/suit.png      base colour for material `suit`
liveries/<team>/<driver>/helmet.png   base colour for material `helmet_paint`
```

Built by `tools/car/build_gen22.sh` (Blender conversion, then the livery
generator). `tools/build_assets.sh cars` runs it alongside the legacy RB6.

## glTF space and nodes

glTF space: +Z forward, +Y up, +X left, metres. Origin at ground level midway
between the axles, laterally centred between the wheels. Every node below is
a direct child of the scene root unless stated.

| Node | Motion | Notes |
| --- | --- | --- |
| `body` | follows chassis | Monocoque, floor, wings, bodywork, suspension arms, cockpit, mirrors, halo. |
| `wheel_fl` `wheel_fr` `wheel_rl` `wheel_rr` | spin about local X, fronts also yaw | Tyre, rim, rim cover, nut, valve. Origin at the wheel centre, no rotation. |
| `upright_fl` `upright_fr` | yaw only, about the wheel centre | Optional. Front brake drums/ducts that steer but do not spin. Origin at the wheel centre. |
| `steering_wheel` | rotates about local +Z | Rim, buttons, paddles, display, LEDs. Origin at the hub; local +Z points from the driver along the column towards the nose. Positive steer (left) turns it anticlockwise as the driver sees it. |
| `drs_flap` | rotates about local +X | Upper rear-wing element and its moving links. Origin on the hinge line. `car.json` gives the open angle. |
| `driver` | follows chassis, morphs with steering and shifts | Four baked steering poses and two finger-pull poses; child `helmet` carries the same targets, with head movement during steering. |
| `paddle_up` `paddle_down` | rotate about local +Y | Children of `steering_wheel`, hinged separately behind the grips. |

## Materials the game addresses by name

| Material | Game behaviour |
| --- | --- |
| `paint` | Base colour texture replaced by `liveries/<team>/paint.png`. One UV atlas, no overlapping islands, 8 px padding at 2048. |
| `suit` | Replaced by `liveries/<team>/suit.png`. |
| `helmet_paint` | Replaced by `liveries/<team>/<driver>/helmet.png`. |
| `wheel_screen` | Player car only: live egui dash. UV (0,0) top-left to (1,1) bottom-right of the visible LCD, landscape. |
| `wheel_leds` | Player car only: rev lights. UV u runs 0 to 1 across the lamps from the driver's left to right. |

Other materials are plain metallic-roughness PBR (`carbon`, `carbon_gloss`,
`tyre`, `tyre_marking`, `rim`, `metal`, `brake`, `rubber`, `glass`, `visor`,
`mirror`, `emissive_red`, and so on). No procedural Cycles node setups are exported.

## `car.json`

The existing `wheels`, `extents` and `faces` keys stay, so `CarGeometry::load`
keeps working. Added keys:

```json
{
  "steering_wheel": { "pivot": [x, y, z], "axis": [x, y, z] },
  "drs": { "pivot": [x, y, z], "axis": [1, 0, 0], "open_angle": 0.0 },
  "cockpit_eye": [x, y, z],
  "driver_poses": [-2.16, -1.08, 1.08, 2.16],
  "driver_shift_poses": ["up", "down"],
  "paddles": { "up_angle": -0.1, "down_angle": 0.1 }
}
```

All in glTF car space. `cockpit_eye` is midway between the driver's eyes.
The six driver/helmet morph targets follow the metadata order: four steering
poses, then upshift and downshift finger pulls. The unmorphed mesh is straight
ahead. The game interpolates adjacent steering targets and adds shift pulls.
Paddles inherit wheel rotation and flick over 0.22 seconds when gears change.

## Budgets

- `car.glb`: at most 300k triangles in total; steering wheel at most 45k;
  driver and helmet at most 35k. At most 60 MB and 42 primitives.
- `car_lod1.glb`: at most 90k triangles and 22 primitives. Retains driver morphs and both paddles.
- Textures: livery 2048 px; everything else at most 1024 px.

## `teams.json`

Source of truth is `tools/car/teams.json`:

```json
{
  "season": 2022,
  "teams": [
    {
      "id": "redbull",
      "name": "Oracle Red Bull Racing",
      "short": "Red Bull",
      "colors": { "primary": "#1E2A5A", "secondary": "#C8102E", "accent": "#FFCC00", "carbon": "#111111" },
      "livery": { "style": "fade", "...": "style parameters" },
      "suit": { "primary": "#1E2A5A", "secondary": "#C8102E", "accent": "#FFCC00" },
      "drivers": [
        {
          "id": "verstappen",
          "name": "Max Verstappen",
          "abbr": "VER",
          "number": 1,
          "helmet": { "primary": "#...", "secondary": "#...", "accent": "#...", "style": "..." }
        }
      ]
    }
  ]
}
```

Liveries use team colours and the team name only. Real sponsor logos are not used.

## Conversion and verification

`tools/car/build_gen22.sh` runs conversion, livery generation and clean Blender
re-import checks. `GEN22_SOURCE`, `GEN22_BAKE`, `BLENDER`, `PYTHON` and its optional
output-directory argument override the default paths. Python requires NumPy and
Pillow; Blender supplies its own NumPy. Blender scripts fail the shell build with
`--python-exit-code 1`.

```sh
tools/car/build_gen22.sh
python3 -I tools/car/test_livery.py
/usr/sbin/blender -b --disable-autoexec --factory-startup \
  --python-exit-code 1 --python tools/car/check_gen22.py -- assets/cars/gen22
/usr/sbin/blender -b --disable-autoexec --factory-startup \
  --python-exit-code 1 --python tools/car/render_gen22.py -- assets/cars/gen22
/usr/sbin/blender -b --disable-autoexec raw/gen22/fp04rb_new_rig.blend \
  --python-exit-code 1 --python tools/car/render_gen22.py -- \
  assets/cars/gen22 --source-comparison
```

The review renderer reads **the exported GLB**, applies Red Bull, Ferrari,
McLaren and Mercedes textures, and writes front, rear, side, top and cockpit
views to a private `/tmp/gen22-previews-*/` directory (or `--output DIR`). The source comparison command writes isolated,
neutral-material steering-wheel and front-wing references to its output directory.
The exported comparison images use the corresponding car-relative camera views.
Cockpit renders hide the driver's own helmet to put the camera at eye level.

The bake files in `raw/gen22/bake/` are compressed NumPy archives named
`paint.npz`, `suit.npz` and `helmet_paint.npz`. Each contains:

| Array | Format |
| --- | --- |
| `position` | 2048 × 2048 × 3 float32, glTF car-space metres |
| `normal` | 2048 × 2048 × 3 float32, unit car-space surface normal |
| `mask` | 2048 × 2048 uint8, 1 inside an atlas triangle |
| `extents` | 2 × 3, car-space minimum and maximum |

Rows use PNG order, with row zero at the top. The generator converts metres to
the normalized coordinates documented in `tools/car/teams.json:livery_styles`.
Packed islands have a 0.008 UV gutter; colours extend twelve pixels beyond
coverage. Folded UV patches are split and repacked before baking. The rasterizer rejects
overlapping texels that describe different surface positions, allowing a few
shared-edge rasterization ties.

The source DRS hinge is at the trailing edge. `open_angle = -0.25` radians about
+X raises the leading edge and flattens the flap, opening its gap to the main
wing. This deliberately follows the actual hinge rather than raising the
trailing edge. Steering uses a hub pivot on the source column at
`(0.056, 1.60, 0.589)` before centering. `cockpit_eye` is the midpoint behind the
new helmet visor at source `(0.0636, 2.075, 0.765)`.

Both GLBs retain the same named motion nodes and all five addressed materials.
The opponent model merges small hardware into carbon but retains screen and LED
materials. The player model uses a generated tileable carbon twill normal and
roughness map, plain matte tyres, and generic yellow compound rings. The new
helmet and posed driver replace the hidden old helmet and footwell shoes.

Hard constraints for this pipeline: disable autoexec on every Blender run;
never execute `rig_ui.py`; exclude `simple_scene`, rig controls and trademarked
art; keep one non-overlapping atlas per replaceable material; preserve the
contract's axes, pivots and budgets. Asset implementation edits are confined to
`tools/car/`, `tools/build_assets.sh`, this document and ignored generated data.
`tools/car/teams.json` remains the roster author's source of truth.

| Surface | Applies | Verification |
| --- | --- | --- |
| Standalone Gen22 build | Yes | Conversion, livery generation, clean GLB re-import |
| `build_assets.sh cars` and `all` | Yes | Gen22 replaces RB22; RB6 command retained |
| Player and opponent GLBs | Yes | Nodes, transforms, materials, triangles, primitives, sizes |
| Team and driver textures | Yes | All roster styles tested; every texture checked at 2048 px |
| Steering, DRS and cockpit metadata | Yes | JSON-to-GLB pivot checks and exported-car previews |
| Rebuild/replacement | Yes | Deterministic output paths overwrite generated assets |
| Game, simulation and packaging | Parallel owner | Consumer verification belongs to the game integration |
| Network providers and connection modes | No | Offline local asset conversion |
| Frozen custom engine | No | Excluded from this pipeline |

## Asset validation record

The 2026-10-30 Blender 5.2.3 build and clean re-import checks passed:

| Artifact | Triangles | Primitives | Bytes |
| --- | ---: | ---: | ---: |
| `car.glb` | 295,731 | 41 | 13,829,652 |
| `car_lod1.glb` | 87,989 | 22 | 4,826,564 |

The player steering wheel uses 44,084 triangles. Driver and helmet together
use 32,998. Both LODs retain all six driver poses and separate shift paddles. All three exported atlases have zero continuous triangle overlaps
and 16.38 pixels of border padding at 2048. Their position bakes also have zero
conflicting texels. Ten teams produce forty PNGs, including twenty helmets.
The eight livery unit tests, Python compilation, shell syntax and asset diff
whitespace checks passed.

All twenty requested Eevee team/view combinations were rendered from the GLB
and visually reviewed. `/tmp/gen22-previews/contact-sheet.png` collects them;
`decimation-comparison.png` compares source and exported steering-wheel and
front-wing close-ups. `drs-open.png` records the source hinge's opening direction.
Generated hashes and geometry metadata are in `raw/gen22/verification.json`.

The final integration pass also verified the generated assets in Bevy: Gen22
gameplay/camera/HUD flows, driver morph and paddle binding on both LODs, and
a two-car cockpit benchmark with six deterministic captures. The regular
workspace unit tests, Clippy and formatting checks passed. Legacy RB6 gameplay
and wide/tall HUD flows remain covered by the renderer checks. A fresh legacy
RB6 conversion and an AppImage rebuild were not part of this pass.

## Importing your PNG artwork

The game reads separate car, suit and helmet PNGs. Export guide layers from the
**current exported model** before painting:

```sh
python3 -I tools/car/import_livery.py --templates raw/gen22/templates
```

This produces transparent `paint-uv.png`, `suit-uv.png` and
`helmet_paint-uv.png` guides. Paint on layers above them, hide the guides, then
export square PNG artwork. Import any combination of the three:

```sh
python3 -I tools/car/import_livery.py --team ferrari --driver leclerc \
  --layout gen22 --paint /path/car.png --suit /path/suit.png \
  --helmet /path/helmet.png
```

Restart the game and choose that team/driver in Race setup. Paint and suit are
shared by the team; helmet artwork applies to the selected driver. RGB artwork
replaces the atlas; transparent RGBA artwork overlays the existing atlas.
Inputs from 256 to 8192 pixels are resampled to the contracted 2048-pixel size.
All inputs are validated before any existing textures are replaced.

Imported textures are also stored in `raw/gen22/liveries/` and reapplied by the
livery generator after every build. `--overrides` changes that source directory
on both import and generation. Remove the relevant source override and rebuild
to return to a generated team texture. Generated assets, source overrides and
PNG guides remain ignored by Git.

**Monoposto 22 PNGs are not Gen22 atlases.** The downloaded
`fp04rb_new_rig.blend` and `fp04rb.zip` describe our base model's original UVs,
which also differ from Monoposto's sheet and from our repacked runtime atlas.
Neither resizing nor renaming those PNGs converts the panel mapping. Transfer
the design to the exported guides, or supply the matching source mesh/UV
correspondence for a proper bake. `--layout gen22` explicitly identifies the
supported import layout; no Monoposto conversion is implied.

## Road contact and cockpit visibility

Wheel hubs and front uprights follow interpolated simulation contact heights;
chassis heave, pitch and roll no longer lift the tyres with the sprung body.
Hub placement accounts for the tilted tyre cylinder's lowest point. Close-range
sun-shadow bias is reduced to keep contact shadows attached.

The wheel has separate material instances for readable carbon and controls,
plus a short-range cockpit daylight fill. Car body materials retain their own
calibration. Only the fifteen top-row lamps respond to RPM/start lights; side
warning lamps retain their geometry and stay off.
