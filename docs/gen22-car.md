# Ground-effect car asset contract

GEN22 uses one shared FP04RB car with neutral stock body paint. Installed per-team
UV0 textures are bound to cloned materials at runtime, without baking team artwork
into the car mesh. Team/driver identities and HUD colours come from
`tools/car/teams.json`. Suits, helmets and cockpit instruments keep their source detail.
This livery layer is stacked on the 2.0 core branch; see [PR boundaries](pr-split.md).

## Safe source preparation

The source is `raw/gen22/fp04rb_new_rig.blend`. Always open it with
`blender -b --disable-autoexec`; never execute its embedded `rig_ui.py`.
The source car points along -Y, up is +Z, left is +X, in metres. Conversion
straightens its approximately 0.4 degree posed yaw and centres it between axles.

Copy only the PNG data from the downloaded `fp04rb` directory into the ignored
`raw/gen22/source-textures/{textures_audi,shadow_maps,material_maps}/` directories.
Run any Python that reads downloaded data with `python3 -I`. Do not run code from
the download. The build losslessly re-encodes copied RGB pixels and AO red channels into
`raw/gen22/pbr/textures/`, removing ancillary PNG data while retaining the
original copied files. It never reads the Downloads directory.
Source material maps are region IDs, not metallic or roughness textures.

```sh
python3 -I - <<'PY'
from pathlib import Path
import shutil
source = Path.home() / 'Downloads/fp04rb'
target = Path('raw/gen22/source-textures')
for folder in ('textures_audi', 'shadow_maps', 'material_maps'):
    (target / folder).mkdir(parents=True, exist_ok=True)
    for image in (source / folder).glob('*.png'):
        shutil.copyfile(image, target / folder / image.name)
PY
tools/car/build_gen22.sh
```

`GEN22_SOURCE`, `GEN22_TEXTURES`, `GEN22_WORK`, `BLENDER`, and the optional output
directory argument select build inputs and output. `tools/build_assets.sh cars`
runs this build alongside the legacy RB6 converter. Conversion scripts live in
`tools/car/`. The archived engine is excluded.

## Generated assets

`assets/cars/gen22/` contains `car.glb`, `car_lod1.glb`, `car_lod2.glb`, `car.json`, and
`teams.json`. AO, normal and cockpit colour images are embedded in the GLBs. Team body artwork stays external. External
team texture directories are optional. Legacy `paint.png`, suit and driver
textures are ignored. See [installation](livery-mapping.md#installation).

The player ceiling is 3 million triangles, including the full evaluated source
steering wheel with no decimation. Main body and wheel-body bakes use up to
4096 pixels; other textures use at most 2048 pixels. Opponent LOD1 is at most
150,000 triangles with textures up to 2048 pixels. LOD2 targets 30,000–50,000
triangles with textures up to 1024 pixels. These are asset ceilings, not frame-rate
claims.

Only the player has a steering wheel, paddles, DRS cap, live screen/LEDs and
finger/shift/thumb morph targets. Opponents retain body, four wheels, front
uprights, DRS flap and a static low-poly driver/helmet silhouette. Cockpit seats,
belts and interior detail are replaced by a dark proxy. There are no opponent
morph targets. `car_lod2.glb` is optional at runtime; missing it leaves LOD1 in
use at every distance. `OPPONENT_LOD_DISTANCE` in `game/src/assets.rs` is 40 m,
with matching 35–45 m Bevy `VisibilityRange` crossfades on each opponent mesh.

`OriginalUV` is the authored UV0. Overlaps and mirrored islands in the source
are intentional and retained. `DetailUV` is a separate metre-scaled UV1 for
carbon twill normals/roughness, without moving the artwork UVs. Only the live
screen, rev strip and separate DRS cap have purpose-built instrument UVs.
The old procedural team generator and runtime atlas are removed. The new
Monoposto 2026 converter targets these original UVs.

## Materials and future artwork

`tools/car/source_materials.py` recreates body materials as glTF PBR.
`tools/car/tyre_bake.py` bakes both source dirt maps and the Pirelli artwork's
alpha through its original `Mapping.001` onto tyre UV0. Rubber stays at
0.035–0.045 linear albedo, roughness 0.7 and dielectric reflectance 0.5 in Bevy.
The build ships neutral rubber, a greyscale marking mask, and opaque compound
textures under `cars/gen22/tyres/`, with `lod1/` and `lod2/` variants.
Only the mask receives compound colour: soft red, medium yellow, hard white.
The material hook selects each entrant's setup compound when attaching its
scene, with separate instances for each compound and source LOD material.
These PNGs use the existing sRGB, mip and BC7 preparation path. No overlay,
additional draw call, paint change or global exposure adjustment is involved.
`tools/car/wheel_bake.py` evaluates the actual source wheel shaders with Cycles,
before transforming geometry. It bakes source colour through emission extraction,
roughness, tangent normals and metallic on each part's own UVs. Nested source
shader inputs, procedural carbon and sticker textures remain the source of truth.
The material's diffuse/base colour is baked without studio illumination, including
conductor colour on metallic parts. Cache keys include the source blend and bake/
pose scripts. Bakes live in ignored `raw/gen22/pbr/wheel-bakes/`.

Core glTF PBR cannot reproduce all Cycles multi-lobe Fresnel/anisotropy or
transmission shaders exactly. Coloured translucent controls retain baked colour
with mild transmission and emission. Live screen/LED/DRS instrumentation keeps
its addressed materials. The static rev strip samples the source lens groups;
Bevy replaces it with per-lamp RPM lighting. Runtime preparation preserves baked
wheel materials and clears the static LED texture when attaching live lamps.

Painted materials are named `source_paint_<part>`, including
`main_body_stickers`, `floor`, `front_wing`, `rear_wing`, `front_covers`,
`rear_covers`, `winglets`, `mirrors`, and `cockpit_foam`. `PARTS` in the material
library is the per-part source of truth. Colour images use sRGB; `shadow_maps`
are non-colour data exported as glTF `occlusionTexture`. AO remains separate
from colour. Source region-ID masks are retained locally for later material
finish authoring and are not mistaken for PBR channels.

The clean hook for future team artwork is the existing per-part material's
`base_color_texture`, on UV0. Replace only that texture with artwork for the
same original part layout. Keep AO, normal, roughness, metallic, UV channels
and geometry intact. The loader uses AssetServer sRGB PNGs, clones materials per team and source
material, and accepts player names plus `_lod`/numeric suffixes. Missing part
files retain embedded artwork. Neither suits nor helmets participate.

Opponent body-panel normals are baked from the detailed player mesh with
Cycles selected-to-active projection on the original UVs. They supplement the
lighter geometry. The player retains its baked cockpit materials; opponent interiors use a dark
proxy. Intermediate review geometry and baked normals live in ignored
`raw/gen22/pbr/`.

## Motion nodes and metadata

Exported glTF space is +X left, +Y up, +Z forward, in metres. Origin is at ground
level halfway between the axles. Motion nodes retain unit scale and identity
rotation at rest.

| Node | Behaviour |
| --- | --- |
| `body` | Chassis, bodywork, floor, wings, suspension, halo, cockpit and mirrors |
| `wheel_fl`, `wheel_fr`, `wheel_rl`, `wheel_rr` | Spin about local X; fronts also yaw |
| `upright_fl`, `upright_fr` | Steer about wheel centres without spinning |
| `steering_wheel` | Local +Z steering rotation, with hub pivot preserved |
| `drs_flap` | Local +X hinge rotation; open angle is -0.25 radians |
| `driver`, child `helmet` | Seven morphs: four steering, two shift pulls, DRS thumb |
| `paddle_up`, `paddle_down` | Children of wheel, local +Y hinges, 0.22-second pull |
| `drs_button` | Child of wheel, travels `[0,0,0.003]` into the wheel |

`car.json` keeps `wheels`, `extents`, `faces`, `steering_wheel`, `drs`,
`cockpit_eye`, `driver_poses`, `driver_shift_poses`, `driver_drs_pose`,
`drs_button_travel` and `paddles`. `faces` comes from the shipped player GLB.
The checker verifies pivots, tyre roundness, real exported morph displacement,
and the player pressed glove's distance from the cap. The player wheel screen
and LEDs retain addressed materials `wheel_screen` and `wheel_leds`. Studio GLB
renders show the placeholder LCD; Bevy replaces it with the live player dash.

Keyboard F and gamepad East drive the same DRS press/release envelope. The LCD
shows unavailable/ready/open states; braking closes DRS. The local helmet is
hidden in cockpit view and restored in external views. GEN22's 4 cm cockpit
near plane preserves forearms at full lock.

## Verification and comparisons

The current export must pass the player, LOD1 and LOD2 contracts independently.
The checker requires a full-resolution player wheel, no opponent wheel/control
nodes or morph targets, separate livery/AO/normal materials, texture limits and
clean re-import. Source/export visual comparisons and Bevy captures are separate
checks; a successful build alone does not prove the material match.

```sh
python3 -I tools/car/test_teams.py
python3 -I tools/car/test_build_gen22.py
python3 -I tools/car/test_check_gen22.py
/usr/sbin/blender -b --disable-autoexec --factory-startup --python-exit-code 1 \
  --python tools/car/test_wheel_bake.py
python3 -I tools/package/test_gen22_assets.py
/usr/sbin/blender -b --disable-autoexec --factory-startup \
  --python-exit-code 1 --python tools/car/check_gen22.py -- assets/cars/gen22
/usr/sbin/blender -b --disable-autoexec --factory-startup \
  --python-exit-code 1 --python tools/car/render_gen22.py -- \
  assets/cars/gen22 --path-traced --output raw/gen22/previews
/usr/sbin/blender -b --disable-autoexec raw/gen22/fp04rb_new_rig.blend \
  --python-exit-code 1 --python tools/car/render_gen22.py -- \
  assets/cars/gen22 --source-comparison --output raw/gen22/previews
/usr/sbin/blender -b --disable-autoexec --factory-startup \
  --python-exit-code 1 --python tools/car/render_gen22.py -- \
  assets/cars/gen22 --lod --output raw/gen22/previews
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
F1_TEST_ASSETS="$PWD/assets" F1_TEST_OUTPUT=raw/gen22/game-review \
  cargo test -p f1-the-game rendered_car_review -- --ignored --nocapture
```

The preview renderer writes `source-full.png` and `source-steering.png` from
the evaluated source blend, preserving source shaders on the wheel and using
the supplied original-UV artwork on the body. `export-full.png`,
`export-steering.png`, `export-rear.png`, and `export-side.png` come from a clean
GLB import with the same cameras and lights. `--lod` and `--lod2` write the
exterior opponent views. `--path-traced` uses the same Cycles engine for source/export
material comparison. The source wheel comparison retains source object
coordinates and actual source shaders. Blender studio comparisons do not prove Bevy
lighting; the ignored renderer check requires a Vulkan display. See README.md
for the isolated Weston harness and other renderer limitations.

| Affected path | Evidence required |
| --- | --- |
| Standalone and root asset builds | Safe conversion and clean GLB re-import |
| Player and opponent GLBs | Triangles, materials, UV channels, AO and projection normals |
| Motion contract | Pivots, tyre silhouette, morphs and cap/glove contact |
| Race/menu identities | Roster parsing and existing selection checks |
| Package preflight | Four required files, embedded artwork |
| Bevy rendering | Real renderer screenshots and control checks |
| Network and archived engine | Not applicable to local car assets |

## Current review results

| Asset | Triangles | Bytes | MiB |
| --- | ---: | ---: | ---: |
| Player `car.glb` | 2,241,266 | 143,871,044 | 137.21 |
| Near `car_lod1.glb` | 139,993 | 19,829,324 | 18.91 |
| Distant `car_lod2.glb` | 44,987 | 9,980,060 | 9.52 |

The full wheel assembly has 1,304,886 triangles: 1,254,070 on the wheel node,
23,040 on the cap, and 13,760 / 14,016 on the paddles. Wheel-body colour,
roughness, normal and metallic bakes are 4096; grips/stickers use up to 2048,
and other controls use 1024. Source geometry is retained before glTF triangulation.

The same-light Cycles comparisons are `raw/gen22/previews/comparison-full.png`
and `comparison-steering.png`. The export retains the dark grips, source
legends and carbon pattern. The source multi-lobe glass and small translucent
controls still differ in highlights; the LCD is intentionally replaced at runtime.
The source static lens colours and game RPM colour sequence are separate states.

Vulkan review on the 4 GB RTX 3050 passes with Half texture detail and full player
geometry. Full texture detail ran out of GPU memory before capture. Set
`F1_TEST_HALF_TEXTURES=1` for the review/control tests on this hardware. This is
not proof that the full-texture configuration fits. Captures are in
`raw/gen22/game-review/`: `cockpit-fixed.png`, `livery-player-*.png` and
`livery-opponent-*.png`. Near/far visibility is asserted against Bevy's actual
view ranges. Opponent texture dimensions are asset limits before runtime mip
stripping.

The final safe build and clean re-import pass all three contracts. Vulkan
`rendered_car_review` passes with both LODs and in a separate asset tree without
LOD2. `rendered_wheel_drs_controls` passes real input, DRS flap/button/thumb
motion and live display checks. Both use Half texture detail on this GPU.
