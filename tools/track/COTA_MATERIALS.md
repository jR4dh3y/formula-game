# COTA material repair — 29 October 2026

The source export points many unrelated surfaces at the same grey asphalt image.
The correct diffuse artwork is embedded in the GLB, but filed in
`KHR_materials_specular.specularTexture`. This explains why geometry repairs alone
left the road, red runoff, blue runoff and white paint looking grey.

Source: `austin-circuit-of-the-americas-2012-layout.zip`,
`source/Untitled_compressed.glb`. The inspected extracted copy was
`/tmp/f1-austin-c-vslb_k23/source/Untitled_compressed.glb`.
Source SHA-256 and complete per-material findings are recorded in
`/tmp/f1-cota-materials-20261029/source-sha256.txt` and `source-materials.json`.

## Explicit repair

`tracks/cota.json` uses `material_base_color_texture`: material-name pattern to
unique embedded **image name**. The converter resolves the named image through an
existing source texture, and only substitutes its base-colour image. There is no
automatic inference from extension names, texture colours or alpha values. Missing
or duplicate image names, absent texture references and overlapping matching rules
fail conversion. Existing alpha/PBR APIs retain their behavior.

Concrete source evidence:

- `fillred` and `rmblx`: source base texture 36; correct texture 132,
  image `roadfillredt-new` (red paint).
- `offtracka_blue`: base texture 36; correct texture 137, image `blue_2`.
- `roadlinewhite`: base texture 36; correct texture 133,
  image `tex_whiteline_a_01_d` (white paint with transparent edges).
- `sanda_red`: base texture 36; correct texture 138, image `re_write1`
  (red/white stripes).
- `roada`/`roada_drs`: base texture 36; correct textures 128/129,
  images `Roada-new`/`Roadadrs-new` (rubbered road and DRS line).
- Pit asphalt, white pit paint, runoff asphalt, grass and two advertisement
  surfaces have corresponding explicit mappings in the config, 17 in total.

The source-image contact sheet is `specular-textures.png` in the evidence folder.
`roadlinewhite` now uses MASK for the remapped white-line image's edge cutout.
`ilya_tents` now uses MASK, matching the already audited `ilya_tents_alpha` artwork.
Existing fence, rail, seat, horizon and truss masks remain; glass rails and tower
glass remain BLEND. Scenery is retained without additional omissions.

## Audit boundaries

This source does **not** have Bahrain's zero base-colour-alpha gravel fault.
Every material factor alpha is 1. `gravela` has legacy texture alpha 49–183;
kerbs `rmbla`/`rmblb` have alpha 0–41; these remain OPAQUE. Masking these channels
would delete valid ground. The source road is already non-metallic with roughness
0.9, so it needs no speculative PBR replacement. Source roughness is retained.

All 163 output materials have decodable embedded base-colour images, unit factor
alpha and double-sided rendering. None can vanish through backface culling. All
576 mesh position, normal, UV and index arrays are identical to the previous
converted asset. Some source faces have degenerate UV triangles (for example,
constant atlas lookups); these are preserved rather than inventing new UVs.
The source normal-map scale is zero on the affected ground materials.

`collision.bin` and `track.json` are byte-identical to the previous assets:

- Collision SHA-256: `847af350607fee616e4cf06353fa58d79aaca52b98831414db0e640cdfbc1d9a`
- Route SHA-256: `88f6c429af13840e9f5970e5b17e21a384018c291188756459aafcd53b977fa8`

## Native visual and driving proof

Evidence root: `/tmp/f1-cota-materials-20261029`, a symlink to
`/home/radhey/.cache/f1-cota-materials-20261029/evidence` because shared tmpfs filled
while rendering. Original assets and frozen test binaries live beside that
on-disk evidence directory. `manifest.json` identifies original/candidate hashes.

Eight isolated Weston/XWayland runs rendered through NVIDIA RTX 3050 Vulkan:
before/after chase, TV pod, spectator and reverse spectator. Every run captured
six sections plus recovery, 56 native 1280×720 PNG captures in total. Each pair
uses the same frozen executable, simulation poses and deterministic timing.
The chase pair uses the original lap-probe executable; all three additional-view
pairs use the same newly built appearance-probe executable. HUD differences
between these executable versions are unrelated to track assets.

Reviewed contact sheets: `comparison-lap.jpg`, `comparison-tv.jpg`,
`comparison-spectator.jpg`, `comparison-spectator-reverse.jpg`. Full-resolution
before/after examples: `proof-lap.png` and `proof-spectator.png`. The latter shows
restored blue runoff, red/white stripes and white lines around the early esses;
chase views show dark rubbered road and restored grass/red paint around Turn 16.
Spectator captures also cover fences, grandstands, trusses and the pit buildings.

The optional `F1_TEST_CAMERA` selector in `game/src/track_probe.rs` accepts
`chase` (default), `cockpit`, `tv`, `spectator`, `spectator-reverse`. It changes
only the camera. The lap and route checks remain identical. The scripts
`lap.sh` and `appearance.sh` in the evidence folder record the exact invocation.

Validation:

- Converter unit tests: 12 passed, including explicit remapping, source mutation
  protection, unchanged alpha/PBR behavior and rejection of invalid mappings.
- Workspace library/binary tests: 26 game and 31 simulation passed; renderer/audio
  tests remain opt-in.
- Clippy all targets with `-D warnings` and formatting check passed.
- Native rendered lap: 145.511 s, 0 contacts, 0 jumps, 0 off-surface fraction,
  worst excess lateral position 0.240 m, maximum tick height step 0.01641 m.
- Both simulated cars complete a lap: RB22 148.272 s, RB6 145.511 s; 0 off-surface
  ticks. All 2,751 route stations have racing ground; maximum ground/route
  difference 0.015 m. All 28 recovery probes pass. Other tracks are absent from
  this private test root and are explicitly skipped.

Limitations: this remains the supplied historical, low-detail model and artwork.
Distant horizon geometry, repeated terrain textures, some aliasing/moiré and old
advertisements remain visible. Fixed spectator offsets partially obscure the
starting grid behind pit roofs; chase/TV cover that section directly. This repair
does not invent missing scenery or modernise the circuit. Spa and other track
configs/assets were not edited by this work.

Installation: the final config was converted again and every regenerated file
matched the reviewed candidate hash. After checking that the live original hashes
had not changed, a same-filesystem staging directory was installed with Linux
`renameat2(RENAME_EXCHANGE)`. The previous directory was moved to
`/home/radhey/.cache/f1-cota-materials-20261029/installed-previous`. Installed hashes
match the rendered candidate; see `installed.json`. No PR or commit was made.

Concurrent-work caveat: the final redundant formatting recheck encountered the
parent's newly declared `mod benchmark` before `game/src/benchmark.rs` existed.
The formatting/workspace/Clippy passes above preceded that unrelated work; rerun
whole-workspace checks after the parent's benchmark module is complete. COTA's
rendered, converter, route and installed-hash validation remains complete.
