# Native and template livery authoring

## Flat UV0 authoring, the active workflow

New liveries are painted flat in 2D on FP04RB's own nine original UV0 layouts,
like the source Audi paint and sticker layers. The main-body layout contains
planar left, right and top views. Place side artwork on the corresponding side
panel. Keep lighting, AO and material response out of the colour image.
`native_paint.py` and the older `<team>.json` projector specs remain available,
but are superseded for authoring. They are not dependencies of the new compositor.
The UV1/direct-template path for existing teams is unchanged.

### Generate the templates

```sh
python3 -I tools/livery/uv_template.py
```

The command opens `raw/gen22/fp04rb_new_rig.blend` using
`/usr/sbin/blender -b --factory-startup --disable-autoexec` under
`flock /tmp/f1-blender.lock`. Embedded `rig_ui.py` is never executed.
Downloaded material-map PNGs in `~/Downloads/fp04rb/material_maps/` are decoded
as image data. No downloaded Python or Paint.NET code runs. The source Audi
`textures_audi_main_body.png`, `textures_audi_main_body_stickers.png`, palette
and `.pdn` documents illustrate the original layered workflow; their colour
pixels are not reused as the new team's paint.

`raw/gen22/uv-templates/` contains the nine 4096-square template PNGs:
`main_body`, `front_wing`, `rear_wing`, `mirrors`, `floor`, `cockpit_foam`,
`winglets`, `front_covers` and `rear_covers`. Each overlays material-map tint,
UV wire, panel names, FRONT arrows, reflection and shared-texel warnings, and
a stretch diagnostic. Green means faces align with the panel's planar view;
red means grazing faces where flat artwork distorts. This cosine diagnostic
is not a guarantee of equal texel density or distortion-free artwork.

`panels.json` records panel name -> part, source chart, UV bbox and concave
boundary polygons, local-to-UV frame, orientation, reflection, front direction,
physical bounds, overlap partners and projection cosine. `masks/` retains exact
triangle coverage for panels and source material-map swatches. Very small
hardware islands have indexed names instead of speculative anatomical labels.
`panel-index.json` is a compact lookup with the largest outer polygon per
panel; use the full masks/index when holes and disconnected fragments matter.
The source-blend fingerprint and per-object UV0 hashes are recorded. Mesh
archives preserve UV0; the generator never repacks or edits it.

Templates flag both overlaps between islands and folds within an island.
Those texels cannot carry different artwork on the surfaces sharing them.
`allow_shared: true` acknowledges that limit for symmetric artwork. The
1024-square overlap sampling can miss subpixel overlaps; inspect the template
and the on-car result as well. `--reuse-meshes` skips Blender only when the
cached export's source fingerprint still matches.

### Paint a team

The first layout is `tools/livery/ferrari-sf26.layout.json`. The compositor is
team independent. A layout needs `schema: 1`, `team`, `source_sha256`, `palette`,
`padding`, and all nine `parts`. Every part has a base `color`, optional `base`
layers, and optional `stickers`, ordered bottom to top.

Base layers accept a `color` or `gradient` with `from`, `to` and a two-point
`axis`. Target a named `panel`, a union of `panels`, a material-map `region`
from the index, and/or a polygon. Polygon `space: "panel"` uses panel-local
coordinates; `space: "uv_px"` uses pixels in the 4096 source layout. Masks
intersect, so a colour boundary cannot spill onto a neighbouring island.
Optional `face_mask` selects an exported flat viewing mask, such as
`main_body:left-side` or `main_body:right-side`. These side masks require a
side-normal cosine above 0.7 and vertical-normal magnitude below 0.6. Use them
to keep Shell, CEVA and UniCredit off the upward-facing sidepod shoulders.
Top, bottom, front and rear masks are also indexed. Geometry is read only by
the template exporter; composition intersects ordinary 2D PNG selections.

Sticker coordinates use the panel's physical reading orientation. `position`
is the centre in normalized panel coordinates, `width` is a fraction of panel
width, and height preserves the cropped artwork's aspect ratio in texture
pixels. Optional `size: [width, height]` overrides both dimensions. `rotation`
is clockwise in that local frame. `flip: "auto"` uses the frame's reflection
exactly once. `flip: "none"` suppresses it; `"x"` adds a horizontal flip.
Stickers clip to their named panel unless `clip: false` is explicit. `variant`
can select `<stem>-<variant>.png` or an alpha-preserving hex tint.

For a curved UV strip or a mark crossing a seam, optional `warp_uv_px` stores a
rectangular grid of flat destination pixel coordinates. The artwork's regular
grid is mapped piecewise affinely onto those 2D points. The grid order explicitly
sets handedness; `position`, `size`, `rotation` and `flip` do not apply to that
warp. This is an image-editor control mesh, with no 3D positions, normals or
projectors in the compositor. Keep separate complementary artwork halves when
the two sides of a source UV seam need different flat footprints.

```json
{
  "image": "shell-colour.png",
  "panel": "sidepod-left",
  "position": [0.43, 0.48],
  "width": 0.22,
  "rotation": 0,
  "flip": "auto"
}
```

```sh
python3 -I tools/livery/compose_livery.py tools/livery/ferrari-sf26.layout.json \
  --logos "$HOME/.cache/livery-2d/ferrari/logos" \
  --out "$HOME/.cache/livery-2d/ferrari/final-4096"

# Existing installer: UV0, 4096 player, 2048 LOD1, 1024 LOD2.
python3 -I tools/livery/install_liveries.py --native \
  --map "ferrari=$HOME/.cache/livery-2d/ferrari/final-4096"
```

Composition produces nine PNGs and nine `.ora` OpenRaster documents, retaining
separate paint-region and sticker layers. PNGs receive nearest-covered-texel
seam dilation. ORA layers retain clean editable artwork; dilation is reapplied
after import. `native-manifest.json` records UV0, all parts, geometry/layout/
panel-index/artwork hashes and output hashes, using the existing installer
contract. `--size 1024` can make diagnostic drafts; installation requires 4096.
The exporter streams one layer at a time into the ORA archive so large layouts
do not retain dozens of full-resolution image buffers in memory.
Composition stages the full set before publishing it. A failed layer leaves
the previous output set intact. Preview validates all nine matching square
PNGs before opening the scene for rendering, so missing paint cannot silently
produce a neutral-material review.

Ferrari review-01 adds artwork corrections alongside the prepared logos.
Regenerate them from the recorded SVG and original launch photograph before
composing the revised Ferrari layout:

```sh
python3 -I tools/livery/prepare_sf26_review_artwork.py \
  --sources "$HOME/.cache/sf26-native/logos" \
  --references "$HOME/.cache/sf26-native/refs" \
  --out "$HOME/.cache/livery-2d/ferrari/logos"
```

`REVIEW-01-SOURCES.tsv` records their provenance. The small photographed badges
retain the supplied photo's limited resolution; enlarged PNG dimensions do not
add detail. UniCredit retains the transparent slash and dark outlined lettering.
The HP halves are reproducible crops of the prepared HP artwork for its UV seam.

For a material-detail comparison, pass `--source-detail` to the Blender preview.
It applies the same metre-scaled carbon normal used by the car exporter only
to dark paint texels, and uses the source tyre marking alpha and its coordinate
graph with an opaque PBR shader.
It does not change the installed car or compound
selection. The default neutral-hardware preview remains useful for paint work.
Concurrent authoring sessions on this workstation run Blender through
`~/.cache/livery-2d/blender_slot.sh`, which owns one of the two shared slots.
Do not start an additional unlocked Blender process while those slots are busy.

Open the ORA documents in Krita or GIMP and save them beside `ora-index.json`.
Editors can drop the embedded custom metadata; that sidecar retains the team
and UV contract. Re-import reads actual layers, including normal-blend groups,
visibility, offsets and opacity, rather than an editor's stale merged preview.
Unsupported blend modes fail explicitly. Flatten those modes to normal layers
in the editor before import.

```sh
python3 -I tools/livery/compose_livery.py \
  --reimport "$HOME/.cache/livery-2d/ferrari/final-4096" \
  --out "$HOME/.cache/livery-2d/ferrari/reimported"
python3 -I tools/livery/install_liveries.py --native \
  --map "ferrari=$HOME/.cache/livery-2d/ferrari/reimported"
```

### Review and verify

```sh
"$HOME/.cache/livery-2d/blender_slot.sh" /usr/sbin/blender -b --factory-startup \
  --disable-autoexec raw/gen22/fp04rb_new_rig.blend --python-exit-code 1 \
  --python tools/livery/render_native_preview.py -- \
  "$HOME/.cache/livery-2d/ferrari/final-4096" "$HOME/.cache/livery-2d/ferrari/final-blender"

flock /tmp/f1-renderer.lock env F1_TEST_ASSETS="$PWD/assets" F1_TEST_TEAM=ferrari \
  F1_TEST_OUTPUT="$HOME/.cache/livery-2d/ferrari/game" \
  cargo test -p f1-the-game rendered_car_review -- --ignored --nocapture

python3 -I -m unittest discover -s tools/livery -p 'test_*.py'
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The renderer needs a Vulkan display. Its native contract covers the player and
both opponent LODs, including UV0 base colour, AO and normals. Restart after
installation. Only the selected team's stale `template.png` files are removed;
other teams and their direct sheets are preserved.

Use `native_contact_sheet.py` with reference, Blender and game paths, including
the opponent captures. Downscale inputs before reviewing; never open raw
renders, textures or photos. Review one bounded contact at a time, at most two
images per tool step, never re-view judged images, and record strict verdicts
in `~/.cache/livery-2d/ferrari/ledger.md`. Build/install/runtime success does not
establish a photographic match. Paint boundaries, colours, sponsor shapes,
sizes and positions remain failures until the corresponding reference agrees.
Only FP04RB's 2022 body geometry may remain as an allowed shape difference.

For the next six teams, reuse this atlas and compositor with a separate
`<team>-<car>.layout.json`, using each team's existing references, logos and
spec in `~/.cache/native-2026/<team>/`. Those teams are not authored by this task.

The flat Ferrari candidate is **not photographically accepted**. In particular,
joining the top HP does not by itself fix its distorted side appearance.
Read the current strict review ledger and final comparison
contacts before treating the layout as finished artwork. The toolchain's
tests and the native renderer contract establish separate technical results.
The authoritative photo corrections retain the white cockpit sidewalls and
surround, black rail text, carbon halo with white partner words, and the
`ZYN 16` nose stack. Earlier instructions to make the surround red or remove
the nose number were superseded. The rear-facing DRS face carries blue IBM;
the upper wing planes stay carbon. CEVA and UniCredit use side-facing masks.
Shell uses a flat control grid to retain its complete silhouette. These marks'
remaining size/position differences and the side HP distortion are artwork
failures, not accepted body-shape differences.

## Retained 3D painter, superseded for authoring

The older Ferrari painter uses the January 2026 SF26 launch photographs on
FP04RB's original per-part UV0. No Monoposto sheet is read by that painter.
McLaren continues to use its existing 2025 direct-template UV1 path below.
`tools/livery/ferrari-sf26.json` records physical colour boundaries and decal
projectors in the source car frame. `ferrari-sf26-logos.json` records public
artwork URLs, source files, derived colour variants and provenance notes.
Brand artwork remains its owners' trademark material; a public download is
not a redistribution licence. The number 16 is isolated from the official launch
photo rather than substituted with a generic font. The source crop has limited
resolution and requires review at game capture size.

The source frame has X left, Y rearward, Z up. The spec's empty transform
removes the source's small posed yaw. Regions are piecewise linear landmark
curves and bounds. Decals use orthonormal physical projection axes, their
original cropped aspect ratio, independent handedness on each side, and
normal/region masks to avoid printing through the car. Original connected UV
charts scope each wing element before source subdivision. Their IDs are
protected by the source blend SHA-256 in the spec. Re-audit the panel IDs
when changing the source; do not bypass the fingerprint.

Halo wordmarks can provide a measured `path` of 3D centreline points. The
projector uses the closest segment, physical arc length and transverse distance
to follow the rail without stretching the artwork across its bounding box.
`up` defines the transverse direction and must not run parallel to a path
segment. The Blender fixture bakes both sections of a bent path and checks
that pixels beyond the artwork footprint retain the base colour.

`native_paint.py` builds emission-only shaders, evaluates the source modifiers,
and bakes with Cycles to the untouched first UV layer at 4096 square. It snapshots
UV0 from the original evaluated mesh before adding panel attributes, then restores
those exact bytes after subdivision reevaluation. This prevents a small UV drift
caused by attribute tagging. It
checks UV0 before/after baking and writes per-object UV hashes plus source, artwork,
painter and spec hashes in `native-manifest.json`. A 24-pixel EXTEND bake margin dilates the colour
across chart gutters. Paint illumination, shadows and AO are not baked in.
Carbon receives a neutral dark base to remove inherited Audi graphics;
existing game carbon detail normals, AO and PBR response remain separate.
Suits, helmets, instruments, compound-aware tyres, geometry and physics are
unchanged. The lightweight native Blender preview uses neutral materials on
nonpainted hardware; the game review is required to assess those materials.

```sh
# Requires rsvg-convert, Pillow, NumPy and SciPy. Public artwork only; no code
# is executed from downloaded files. Derived PNGs replace atomically.
python3 -I tools/livery/prepare_native_logos.py \
  tools/livery/ferrari-sf26-logos.json \
  --directory "$HOME/.cache/sf26-native/logos" --download

/usr/sbin/blender -b --factory-startup --disable-autoexec \
  raw/gen22/fp04rb_new_rig.blend --python-exit-code 1 \
  --python tools/livery/native_paint.py -- tools/livery/ferrari-sf26.json \
  --logos "$HOME/.cache/sf26-native/logos" \
  --out "$HOME/.cache/sf26-native/baked" --size 4096

python3 -I tools/livery/install_liveries.py --native \
  --map "ferrari=$HOME/.cache/sf26-native/baked"

/usr/sbin/blender -b --factory-startup --disable-autoexec \
  raw/gen22/fp04rb_new_rig.blend --python-exit-code 1 \
  --python tools/livery/render_native_preview.py -- \
  assets/cars/gen22/liveries/ferrari "$HOME/.cache/sf26-native/blender"

F1_TEST_ASSETS="$PWD/assets" F1_TEST_TEAM=ferrari \
  F1_TEST_OUTPUT="$HOME/.cache/sf26-native/game" \
  cargo test -p f1-the-game rendered_car_review -- --ignored --nocapture
```

The native installer validates the manifest and all nine 4096 PNGs before
writing. It installs 2048 LOD1 and 1024 LOD2 variants and removes only the
selected team's three `template.png` files, because those files take priority
over native parts in a template-enabled model. Restart the game after
installation. Other teams' sheets remain intact; no team-specific Rust branch
or exporter rebuild is required. The renderer check asserts native body base
colour, AO and normals on UV0 at player, LOD1 and LOD2, while retaining its UV1
assertions for direct-sheet teams.

`native_contact_sheet.py` consumes a JSON list with `name`, `reference`,
`blender`, and `game` paths plus optional `reference_box`, `blender_box`, and
`game_box` crops for comparable framing. It writes one
bounded JPEG per angle and four-angle pages at 1280 by at most 1160. Use it
before viewing images; never open full-resolution renders or textures during
review. The local manifest is `~/.cache/sf26-native/final-contact-manifest.json`.

Detailed photo measurements, strict visual verdicts, render iterations and
check logs are in `~/.cache/sf26-native/{spec.md,palette.json,ledger.md}`.
Passing the bake, installation or renderer test does not establish photographic
livery parity. The ledger is the authority for visible residual differences.

The current native candidate is **not photographically accepted**. Visible
differences remain in the white cowl/cockpit perimeter, projected cover marks,
sponsor spacing and small halo/chassis labels. FP04RB's 2022 nose, raised fin,
sidepods and wing geometry are separate shape differences. Do not treat paint
errors as permitted shape residuals. This work does not establish the requested
indistinguishable livery match.

## 2025 direct-template path, retained for McLaren

The Ferrari SF25 trial below is historical and was rejected for its visible
colour and logo mismatch. Use native SF26 painting above for Ferrari. The
shared UV1 exporter/runtime path remains active for McLaren.

GEN22 now authors a second UV set named `livery` directly on FP04RB paint
meshes. It samples the supplied Monoposto **2025** sheet without warping or
rebaking sponsor artwork. UV0 remains the original source layout. The 2026
original-UV converter below is retained as a legacy path; its distortion
measurements do not describe the 2025 mapping.

The primary sheet is `raw/gen22/monoposto-sources/2025/ferrari-sf25-a.png`,
Hamilton #44. The alternate `ferrari-sf25-b.png` is Leclerc #16. Credits are
in that directory's `SOURCES.txt`. The runtime roster still describes the
existing 2022 entrants; this change installs artwork and does not rename them.

### Build and install

```sh
# Opaque white-background wireframe, unlike the transparent 2026 reference:
python3 -I tools/livery/template_islands.py \
  --template raw/gen22/mapping-reference/2025-template.png \
  --out raw/gen22/livery-mapping/2025 --dark-wireframe --thresh 220

# Default is 2025. Keep downloaded Blender Python disabled on every invocation.
GEN22_TEMPLATE_YEAR=2025 tools/car/build_gen22.sh
python3 -I tools/livery/install_liveries.py --template-year 2025 \
  --map ferrari=raw/gen22/monoposto-sources/2025/ferrari-sf25-a.png \
  --map mclaren=raw/gen22/monoposto-sources/2025/mclaren-mcl39-a.png

/usr/sbin/blender -b --factory-startup --disable-autoexec \
  raw/gen22/fp04rb_new_rig.blend --python-exit-code 1 \
  --python tools/livery/render_template_preview.py -- \
  raw/gen22/monoposto-sources/2025/ferrari-sf25-a.png /tmp/sf25-preview
```

`template_islands.py` finds 35 connected outline groups. A connected group
can contain several printed surfaces; island 19 contains four separate wing
ribbons. `template_2025.py` records their own seams. Body boundaries reuse the
recorded anatomical source loops, then solve with positive graph weights onto
the 2025 panels. Whole polygons own one panel, including separate cover halves
at the spine. Both sides use independent template coordinates and handedness.
Interior side-view scale constraints reduce shear around the broad printed
areas; the anatomical boundary remains pinned.
Nose, cockpit rim, halo circumference, mirrors, wheel discs and floor lips
use their corresponding charts. UV0 is asserted bit-for-bit unchanged during
authoring. Small lettering is still distorted by the different FP04RB shape.

### Island to FP04RB correspondence

IDs are from `raw/gen22/livery-mapping/2025/islands.json`; image coordinates
are in the unmodified 2048 square template. Right and left refer to the driver.
Unassigned surfaces remain carbon; they do not borrow another part's logos.

| Island | Template surface | FP04RB counterpart and status |
|---|---|---|
| 1 / 2 | Engine-cover left / right | `body_object`, split at spine; direct mesh solve |
| 3 / 22 | Small chassis/cover caps | No confident separate counterpart; unused |
| 4 / 11 | Front outer endplates | `front_wing_mixer`, separate right/left faces |
| 5 | Rear DRS rear-facing strip | `rear_wing_no_mod._drs` and fixed tips; IBM on Ferrari |
| 6 / 7 | Rear outer endplates | `rear_wing_no_mod`, B&O panels on Ferrari |
| 8 / 9 | Long rear chassis/wing-support panels | Unassigned; source rear supports remain carbon |
| 10 | Rear upper flap and main-plane top | Split at image row 583 into two physical elements |
| 12 | Paired longitudinal body/underbody pieces | No separate source chart confidently identified; unused |
| 13 / 16 | Mirror housings | `mirror_main_body_at01`, independent sides |
| 14 | Front-wing central bridge/support | Source support remains carbon; unused |
| 15 | Rear main-plane underside | `rear_wing_no_mod`, separate lower-facing surface |
| 17 | Cockpit insert | Source cockpit foam remains carbon; unused |
| 18 | Cockpit surround | `body_object` cockpit rails |
| 19 | Four front-wing ribbons | Four physical elements of `front_wing_mixer`; normal-facing separation |
| 20 | Rear inner endplates, paired | Split at image column 928, independent inner faces |
| 21 / 23 | Front inner endplates | `front_wing_mixer`, independent faces; AWS on Ferrari |
| 24 / 35 | Floor-edge right / left panels | `floor_rb_mixer` outer lips; source lip is much thinner |
| 25 / 34 | Sidepod right / left | `body_object`, independent mesh solves; vertical template reflection |
| 26 / 32 | Small aero fin faces | No confident source counterpart; unused |
| 27 | Nose and chassis | `body_object`, longitudinal stations with transverse wrap |
| 28 | Small longitudinal red insert | Unassigned; source inserts remain carbon |
| 29 / 33 | Long thin outer/body-edge strips | Unassigned; the tiny source edges cannot carry these logos legibly |
| 30 | Wheel-cover disc | All four `*_wheel_cover_*` meshes, common disc layout |
| 31 | Halo | `body_object` halo charts, two arms and circumferential wrap |

Unassigned counterpart descriptions are provisional. The Monoposto 2025 3D
mesh is not available, so the template alone cannot confirm every small part.
These rows are not a claim of complete small-panel correspondence.

### UV and material contract

Bevy supports UV0 and UV1. The exporter retains `OriginalUV` as TEXCOORD_0
and `livery` as TEXCOORD_1 in the player, LOD1 and LOD2. Painted carbon detail
normals formerly using a third temporary `DetailUV` are baked into the
OriginalUV tangent frame before that temporary layer is removed. AO stays on
OriginalUV. Unpainted composite keeps its metre-scaled detail coordinates in
UV1. The normal bake and LOD-normal projection run before export.
Paint UV1 seams are split geometrically before collapse decimation, preserving
corner normals. This prevents LOD triangles from bridging unrelated template
panels. A face attribute records each source panel's ownership. After
subdivision and each collapse, UV1 corners return to a three-pixel inset of
that panel; UV0 and unpainted detail coordinates are untouched. Brake-cover
fins retain their source carbon material; only the four
wheel-cover meshes use the wheel-disc artwork.

`car.json` records `livery_template: 2025`. The game selects `template.png`
on `UvChannel::Uv1` only for a model with that metadata. Each team and source
material gets its own instance. Base colour is white so the input sheet
colours survive unchanged; AO, normal and PBR settings are preserved.
Older asset packages continue using original-UV per-part textures.

The installer retains every pixel inside a template island at its original
coordinate, extends island edges 64 pixels into gutters, and writes
`liveries/<team>/template.png`, `lod1/template.png` at 2048 and
`lod2/template.png` at 1024. It loads as sRGB. LOD files fall back to the root
sheet. Missing direct sheets fall back to existing per-part artwork or the
embedded source artwork. Running the legacy 2026 installer removes its own
three direct-sheet files after conversion so they cannot shadow the new
per-part textures. Restart the game after installation.

### Validation and local visual evidence

Renderer review command, including both opponent LODs:

```sh
F1_TEST_ASSETS="$PWD/assets" F1_TEST_TEAM=ferrari \
  F1_TEST_OUTPUT=/tmp/sf25-game \
  cargo test -p f1-the-game rendered_car_review -- --ignored --nocapture
```

The review asserts UV1 base colour, UV0 AO/normals and mesh UV0/UV1 presence
for player and both opponent LODs. It also checks near/far visibility ranges.
Its 13 captures include both side views, front, overhead, wing closeups,
cockpit, and opponent LOD1/LOD2.
Full check results and visual verdicts are recorded in the local work ledger
`~/.cache/sf25-hillclimb/ledger.md`. Reference URLs and credits are in
`~/.cache/sf25-refs/SOURCES.tsv`.

### Final local results, 2026-11-01

The final exports pass `check_gen22.py`, including clean Blender re-import,
paint UV0/UV1 presence and domain, UV0 AO/normals, and player/LOD contracts.
A corrected glTF audit uses sheet-oriented V directly. It found zero triangles
with vertices in different islands and zero cross-panel or empty-gutter hits
at 16 sampled locations per triangle after gutter padding. This finite sample
check does not establish distortion-free mapping.

Both full-texture `rendered_car_review` runs passed on the RTX 3050 Vulkan
backend in an eight-car scene, one run for Ferrari and one for McLaren. Each
wrote 13 captures and passed the near/far opponent visibility assertions.
`cargo test --workspace --lib --bins` passed 108 tests with 14 ignored;
Clippy with `-D warnings`, Cargo formatting and `git diff --check` passed.
Python discovery passed 53 tests, with three Blender-only cases skipped there
and passed separately in Blender. Source tyre and wheel bake regressions also
passed in Blender.

Final reviewed contact sheets are local artifacts:

- `~/.cache/sf25-hillclimb/final-ferrari-contact.png`: reference, Blender and Vulkan.
- `~/.cache/sf25-hillclimb/final-wings-lods-contact.png`: wings, cockpit and both opponent LODs.
- `~/.cache/sf25-hillclimb/final-mclaren-contact.png`: shared second-team mapping in Blender and Vulkan.

The complete source-file list, log paths and remaining mismatches are in
`~/.cache/sf25-hillclimb/final-report.md`. Runtime/export acceptance passed;
full photographic matching and complete small-panel identification remain unmet.

### Known visual differences

FP04RB's narrow nose, raised cover fin, sidepod shoulder and thin floor lips
still differ from the real SF25. Nose artwork and floor sponsors are smaller,
the cover/sidepod colour boundaries differ, and large artwork can bend around
the fin. McLaren's Chrome mark exposes the same cover-shape limitation.
The mapping is shared across teams. The provisional unused small-panel rows
above remain unresolved; this is not complete photo parity.

## Legacy 2026 original-UV converter


`tools/livery/` converts livery sheets painted on the official Monoposto 2026
template (2048 × 2048) into textures on the FP04RB model's **original** UV
layout. It writes one PNG per paint part (`main_body`, `front_wing`, `rear_wing`,
`mirrors`, `floor`, `cockpit_foam`), on the same layout as the
`material_maps/` and `textures_audi/` files that ship with FP04RB.

Monoposto's 3D car is not available and its shape differs from FP04RB, so this is
a transfer between two cars. The mapping is derived only from the reference
template's UV islands and FP04RB geometry. Community sheets are never used to fit it.

## How it works

1. **Template islands** (`template_islands.py`): the template is segmented into
   58 UV islands. Island 26 is the centre column: engine-cover halves (rear at the
   top, split along the spine), cockpit surround and nose (tip at the bottom).
   Islands 21/22 are the right/left sidepods, 4 the halo, 34/36/37 the front-wing
   elements, 5/15 the outer and 11/12 the inner front endplates, 43–47 the rear-wing
   surfaces, 35/41 the outer and 20/30 the
   inner rear endplate faces, 27/28 the chassis sides beside the cockpit and 33/39 the
   round wheel covers. Island 42 belongs near the cockpit/halo, not on a wing,
   and is currently unused. Image-right is the car's left in the front-wing strips.
2. **FP04RB texels** (`bake_fp04rb.py`): every texel of each paint texture is
   rasterised from the `3D_parts_mixer` OBJs (exact triangle coverage, seam-aware)
   to a 3D position and normal. OBJ space: +X forward, +Y up, +Z the car's right.
3. **Body patches** (`make_patches_2026.py` -> `patches_2026.json`): FP04RB's own body
   UVs are three planar projections (island 19 = right side, 17 = left side, 18 = top view of
   the cockpit, sidepod tops and nose). Each body patch is a closed loop of control points
   written in those views: the centreline, the floor border, the shoulder crease between
   engine cover and sidepod, the sidepod front, the cockpit rim. Every loop anchor is
   pinned to a panel boundary on the template. No sponsor location constrains the solve, so
   every team sheet uses the same mapping. Patches: engine cover half -> island 26 upper
   half, sidepod -> 21/22, cockpit surround + nose -> island 26 lower half. The spec
   describes the car's right side; the left is mirrored from the template's own left
   islands, so asymmetric artwork stays on its painted side.
4. **Surface solve** (`build_map_v3.py`, `refine_mesh.py`): a graph over all body texels
   (UV 4-neighbours plus seam links between UV islands that touch in 3D) carries each loop
   as a shortest 3D path. Paths may step diagonally in UV and are then made 4-connected
   again. A 4-neighbour-only path produced coarse staircases that showed on the car as
   sawtooth patch borders. The interior is a cotangent (surface-metric) harmonic fill. The
   engine cover is re-solved on welded OBJ triangles (`refine_mesh.py`) and baked back
   onto the texels. This reduces tail folding. The halo keeps the circumferential unroll
   from `build_map_2026.py`. Texels in no patch (cockpit rim, airbox front) copy the
   nearest patch texel and are then harmonically smoothed.
5. **Wings** (`wing_map.py`, called by `build_map_2026.py`): the FP04RB shadow maps
   guide chart identification; the baked positions and normals define the correspondence.
   Front-wing strips 34/36/37 follow the visible upper envelope by chord arc length.
   Overlapped lower surfaces stay carbon rather than repeating logos through the slots.
   Rear-wing charts separate main-plane top (43), DRS forward face (44/45), DRS rear
   face (46) and main-plane underside (47). Each row follows the actual surface arc,
   including near-vertical faces. Stacked strips skip the template gaps. Rear span runs
   vertically on the template, with car-right at the top. Both faces of each front and
   rear endplate use their own panel outline and orientation.
6. **Other parts** (`build_map_2026.py`): wheel covers have no OBJ. Their glTF UVs (measured on
   `car.glb`) are a planar disc centred at (0.5, 0.5) with radius 0.455. That disc is
   scaled linearly onto template discs 33 (front) and 39 (rear), rim to rim. The template
   hub opening and upright pieces outside the disc stay carbon. Mirrors take the body's
   dominant colour; floor, cockpit foam and winglets are carbon. The winglet fill also
   stops the embedded Audi artwork leaking into converted teams.
7. **Conversion** (`convert_2026.py`) samples the sheet bilinearly through the stored
   map (`raw/gen22/livery-mapping/map_v3.npz`, git-ignored). It never interpolates
   across a chart seam and needs no Blender. It writes nine parts: `main_body`,
   `front_wing`, `rear_wing`, `mirrors`, `floor`, `cockpit_foam`, `winglets`,
   `front_covers`, `rear_covers`.

## Usage

```sh
# one-time: copy FP04RB's OBJs (untrusted download data) into the work dir
mkdir -p raw/gen22/livery-mapping/obj
cp ~/Downloads/fp04rb/3D_parts_mixer/*.obj raw/gen22/livery-mapping/obj/
tools/livery/build_2026.sh                       # islands, bake, base map, body patches, checks (~4 min)
# After a wing-only mapping edit, preserve the existing body solve:
python3 -I tools/livery/update_wings.py

python3 -I tools/livery/convert_2026.py sheet.png --out out/mclaren
python3 -I tools/livery/convert_2026.py sheets/ --out out/      # batch: out/<stem>/

# visual check (Blender): renders side, top and three-quarter views
/usr/sbin/blender -b --factory-startup --disable-autoexec --python-exit-code 1 \
  --python tools/livery/render_preview.py -- out/mclaren /tmp/livery-preview
```

`make_test_sheet.py` makes a gridded test sheet with one colour per island and
asymmetric "F" glyphs, for checking placement and mirroring.

## Installation

Run from the repository root with isolated Python. Team IDs come from
`assets/cars/gen22/teams.json`. Repeat `--map` to install several teams. Every
mapping is validated before conversion begins; unknown or duplicate IDs fail.

```sh
python3 -I tools/livery/install_liveries.py \
  --map mclaren=raw/gen22/monoposto-sources/mclaren-austin-2026.png
```

The installer calls `convert_2026.convert()` at 4096 and writes
`assets/cars/gen22/liveries/<team_id>/<part>.png`, plus 2048-pixel `lod1/`
and 1024-pixel `lod2/` variants for opponents. Missing variants fall back to the
root part file. The game replaces only matching
`source_paint_*` base-colour textures on player and opponent models, including
LOD material suffixes. `main_body.png` replaces `main_body_stickers` in full;
there is no second Audi sticker layer. AO and normal maps stay separate. Teams
have separate material instances. Missing files keep the embedded Audi artwork.
The converter writes all nine parts, including the wheel covers and the winglet carbon
fill. Legacy `paint.png`, suit and helmet files are ignored.
Restart the game after installing artwork.
The neutral mirror fill uses the dominant colour sampled across all mapped parts.
Reinstalling after a wing-map edit can therefore slightly change that colour even
when the mirror mapping is unchanged.

## Measured quality

Measured on the 2026-10-31 build. `build_2026.sh` runs `check_stretch.py`,
`check_handedness.py` and `audit_mapping.py`. Anisotropy is the singular-value ratio of
template px per surface unit, p50/p90:

| Surface | Anisotropy p50/p90 | Not mirrored | Folded / collapsed |
|---|---|---|---|
| Engine cover R / L | 2.50/3.80, 2.47/3.16 | 99% | 1.1–1.3% / 1.3% |
| Sidepod R / L | 1.33/2.21, 1.25/2.08 | 97% | 0.3% / 2.8–2.9% |
| Cockpit + nose R / L | 1.62/5.95, 1.65/10.02 | 94%, 92% | 2.0–2.1% / 3.7–6.9% |
| Halo | 3.02/12.36 | 99% | 2.2% / 1.8% |
| Front wing top / middle / main | 2.14/3.74, 2.26/5.11, 2.14/5.04 | 100% | <0.02% / <0.01% |
| Front outer endplates | 2.13–2.14 / 5.23–5.36 | 97% | 2.73–2.83% / 0.11–0.12% |
| Front inner endplates | 2.14–2.16 / 4.16–4.20 | 100% | <0.03% / <0.03% |
| Rear main-plane top | 1.11/2.15 | 100% | 4.13% / 0.09% |
| DRS forward face, two strips | 3.36–3.37 / 3.55–3.57 | 100% | <0.02% / <0.06% |
| DRS rear face | 1.41/4.33 | 100% | 1.49% / 0.05% |
| Rear main-plane underside | 1.54/8.66 | 97% | 4.69% / 0.06% |
| Rear endplates (4 faces) | 1.53–1.68 / 2.51–2.99 | 100% | 0% / 0% |

Body coverage: 914,520 of 936,080 baked body texels map to template coordinates. The rest
is carbon or rim fill. 87,926 paint texels outside all patches (cockpit rim and airbox
front) use the smoothed nearest fill. Visual check: install the McLaren and Ferrari
sheets and run the `rendered_car_review` renderer test (README.md). It captures
both sides, front, rear, cockpit and opponent LOD1/LOD2 views.

## Known limits

- FP04RB is a 2022-shape car and Monoposto's 2026 car is narrower, so this is a transfer
  between two different shapes, not an exact match. Artwork on the sidepod undercut and
  sidepod top is stretched. The rows below the sidepod logos bend round the undercut
  bulge. Patch borders still follow FP04RB's panel creases, which are not straight.
- Cockpit/nose p90 anisotropy (6–10) is concentrated on the steep cockpit rim and nose
  tip. Small lettering there is sheared.
- Front-wing lettering still bends and shows waviness near slots and curved edges.
  The envelope shares artwork across a different set of physical elements than the
  source car, so this is not an exact per-element UV match.
- Rear main-plane top and underside retain 4.13% and 4.69% folded sampled area.
  Each rear region has less than 0.1% collapsed sampled area; the underside still has
  p90 anisotropy 8.66. The audit does not imply distortion-free artwork.
- The final map samples 22 of the 58 template islands: 4, 5, 11, 12, 15, 20–22, 26,
  30, 33–37, 39, 41 and 43–47. Islands with no confident FP04RB counterpart are not
  used: 1–3, 6–10, 13–14, 16–19, 23–25, 27–29, 31–32, 38, 40, 42 and 48–58.
  These include the mirror, beam-wing and
  winglet artwork. They also include the chassis-side panels 27/28 (McLaren's "intel"),
  which FP04RB's body patches cover from island 26 instead. Mirrors are body colour; winglets, floor and cockpit foam are carbon.
- Front/rear wheel-cover assignment (33 front, 39 rear) is not confirmed by the template.
  Both supplied sheets paint identical discs, so it has no visible effect for them.
- The 2024 template is not supported. The 2025 template uses the direct path above;
  it does not use this 2026 converter or its mapping files.
