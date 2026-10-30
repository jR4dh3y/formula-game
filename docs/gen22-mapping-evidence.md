# Gen22 mapping completion record

Status: mapping corrections and generated-asset verification completed; visual acceptance still fails the stretch/cropping targets. No visual approval claimed.

## Constraints

No commits, pushes, UI, simulation or archive changes. Preserve source blend,
source artwork, existing edits, morphs, DRS and budgets. Every Blender command
uses `--disable-autoexec`. Work stays in root `tools/car` and generated assets.

## Provenance and implementation

- Official guide: https://www.gabama.com/monoposto/livery-template.zip,
  `raw/gen22/mapping-reference/2026-template.png`. The profile records its SHA-256.
  The 2025 layout differs and is unsupported.
- Model: original FP04RB Blender file, unchanged. Original body, individual wing,
  DRS, halo and mirror charts are inspected before subdivision.
- `monoposto-modern.json` stores original chart bounds, mesh-face seam selectors
  in original UV space, and official-guide landmarks/contours.
- `author_uv.py` measures actual mesh-chart cross sections. `Monoposto2026`
  stores the authored loop UVs. `RuntimeAtlas` stores shipping UVs. Both survive
  evaluation/decimation. The same rasterizer emits position and source-UV maps.
- Old target-texel XYZ projection has been removed.
- Converter rejects missing source UVs and profile/bake mismatch. Rebuilds
  regenerate saved imports from original PNGs, checking their saved digest.

## Applicable paths

CLI single import, recursive batch, persistent rebuild, generated teams,
player, opponent LOD, moving DRS, inward cockpit, source preservation, source
layout contract and documentation apply. UI changes, network transport,
simulation, legacy workspace and native packaging do not apply to this work.

## Evidence locations

- `/tmp/gen22-mesh-map-final-evidence/`: original mesh UV overlays and authored
  UV overlays on the official guide, plus aggregate triangle diagnostics.
- `raw/gen22/bake/correspondence.json`: per-chart and per-patch topology metrics.
- `/tmp/gen22-mesh-map-final-build.log`: full rebuild and asset contract checks.
- `/tmp/gen22-python-final.log`, `/tmp/gen22-rust-test.log`,
  `/tmp/gen22-clippy.log`, `/tmp/gen22-build.log`: focused and workspace checks.

## Limits to resolve

The original charts include holes, small degenerate faces, and UV folds.
The transfer still records local reversed/collapsed triangles on body and halo
patches. Aggregate sign counts include intentional left/right chart mirroring;
per-patch metrics compare against original UV orientation. These are defects to
inspect, not a clean foldover pass. Small airbox vanes have heavily degenerate
original UVs. Mapping cannot be described as exact or approved until close Bevy
captures establish sponsor placement. The original Monoposto mesh was not in
the official template archive.

## Orientation follow-up

Direct review of the original source objects exposed the reversed longitudinal
orientation and distinct vertical direction of the original left/right charts.
Both are now explicit in the profile, independently of runtime packing. The
front endplate source charts use UV U for physical height and V for longitudinal
position; their guide transfer swaps those axes. Paired front wing halves use
half an island each, instead of repeating a whole sponsor strip.

Reproduce direct original-object review without rebuilding the runtime atlas:

```sh
blender -b --disable-autoexec raw/gen22/fp04rb_new_rig.blend \
  --python-exit-code 1 --python tools/car/review_correspondence.py -- \
  --source raw/gen22/monoposto-sources/mclaren.png --output /tmp/gen22-source-review
```

## 2026-10-30 implementation follow-up (Sol)

The prior orientation follow-up above describes the rejected intermediate cages.
The final profile now pairs the original yellow (+X/physical left) chart with
**guide-right**, and red (-X/physical right) with **guide-left**. Nose points down
on the guide; the inner edge is car-top. Nose-side mapping starts at guide y=.525.
The nose ridge takes a narrower central interval, with side charts covering the
rest of the arc; this is an approximation, not a measured 3D arc-length solution.

Implemented in `tools/car/author_uv.py`, `monoposto-modern.json`,
`prepare_gen22.py` and regression tests in `test_author_uv.py`:

- Wing strips measure actual source cross sections along original V instead of
  using bbox U. Non-body axis swaps now actually apply to sampled coordinates.
- Rear endplate orientations are independent per chart; mainplane/DRS use bowed
  guide columns. A focused DRS rebake corrected guide-line contamination without
  another whole-model conversion.
- Cockpit rails use their separate .416-.452/.548-.584 bands. Halo charts sample
  one plain colour. Original no-UV carbon vanes stay carbon, with obsolete chart
  evidence removed.
- LOD body decimation splits UV/material seams and collapses inside chart space,
  carrying physical coordinates as an interpolated point attribute. Driver,
  helmet and DRS geometry stays exact; other parts absorb the remaining budget.
  This preserves the common paint/suit/helmet atlases without repacking them.
  Small remaining body folds trigger a retry with their source vertices and one
  neighbouring edge ring protected; no checker tolerance was changed.

No-GPU final original-chart results (`/tmp/gen22-final-coverage.log`,
`/tmp/gen22-final-stretch.log`):

| Painted object | Off-island coverage | Surface anisotropy p50 |
| --- | ---: | ---: |
| body | 2.2% | 2.13 |
| front wing | 2.8% | 1.83 |
| rear wing | 1.9% | 1.75 |
| DRS | <0.1% (3 pixels) | 2.79 |
| mirrors | 0.2% | 5.23 |

All five meet the <3% coverage target. None meets the p50 <1.6 stretch target.
The mirror and rear-endplate contours reduce off-island sampling but increase
local distortion. Some Dropbox letters still cross the original endplate notch
in the source-chart rewarp. Body patch folds/collapses and cockpit/airbox fitting
remain visible in diagnostic maps. These source-correspondence defects are
separate from the runtime atlas overlap contract. **No visual approval claimed.**

Final source blend SHA-256:
`232827ee55d9d0140be52e511618db8b62e91e575055cb7099054b2d56128c20`.
The official guide retains the SHA-256 recorded by the profile.


Final verification:

- `python -m unittest discover -s tools/car -p 'test_*.py'`: 33 passed.
- `cargo test --workspace --lib --bins`: 56 game and 32 sim tests passed;
  11 renderer/long-running tests ignored by the normal command.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `/tmp/gen22-corrected-contract.log`: **GEN22 CONTRACT PASS** for both actual
  GLBs, including required nodes, six morph targets, paddles, materials, every
  team, and zero body/DRS/suit/helmet UV intersections. No contract weakened.
- `/tmp/gen22-corrected-export.log`: full 296,309 / LOD1 87,894 triangles.
- Active McLaren, Ferrari and Mercedes paint regenerated from their preserved
  sources; alternate Mercedes 54 and Porsche converted separately in
  `raw/gen22/converted/`. Original PNGs and source blend were not edited.
- Final Bevy eight-view sets (cockpit, front, left, right, rear, opponent front,
  opponent side, opponent rear): `/tmp/gen22-final2-bevy-mclaren/`,
  `/tmp/gen22-final2-bevy-ferrari/`, `/tmp/gen22-final2-bevy-mercedes/`,
  `/tmp/gen22-final2-bevy-mercedes54/`. Alternate Mercedes uses an isolated asset
  root; the active Mercedes remains the Malay sheet.

The initial full-rebuild log includes a failed old LOD export check, before the
final chart-space collapse/retry fix. The focused DRS rebake, corrected export,
texture regeneration and final strict contract are recorded separately in
`/tmp/gen22-focused-drs.log`, `/tmp/gen22-corrected-export.log`,
`/tmp/gen22-corrected-livery.log` and `/tmp/gen22-corrected-contract.log`.
Use these final artifacts rather than the intermediate failed build check.
