# Silverstone material audit

Source: `silverstone-circuit-2024-layout.zip` → `source/silverstone.glb`
(Blender glTF I/O 4.4.56 export of an Assetto-style track). All 70 source materials
are `OPAQUE`, `metallicFactor` 0, `baseColorFactor` 1 and `doubleSided`. So the
Bahrain fault (a MASK material with zero alpha) and its metallic road do not occur
here. Every primitive has `TEXCOORD_0` and `NORMAL`, there is no texture transform,
and the converter writes all materials double-sided. Collision and route outputs are
byte-identical to the previous install, because only visual materials changed.

## Faults and repairs (`tracks/silverstone.json`)

| Material | Evidence | Repair |
| --- | --- | --- |
| `barriers1.*`, `metals1.*`, `walls2.001`, `box1_multimap.001`, `tile1.001`, `tile1B.001` | The base slot holds tiling AC detail maps (`concrete_detail4`, `metal-detail2`, `concrete-detail2`, `box_detail1`, `metal_detail`) with saturation near 0, which render as flat white/grey. The real colour atlases (`barriers1`, `metals1`, `walls2b`, `box1`, `tile1`, `tile1B`) sit in `KHR_materials_specular.specularTexture`. The archive's loose folder holds only `<atlas>@channels=A.png` for these, so the importer used just their alpha for specular. `box1.001` uses the `box1` image as its normal base colour. | `material_base_color_texture` selects the atlas. The material stays `OPAQUE`, so the atlas alpha (an AC spec/detail mask) is discarded. |
| `grass.001` (≈152k m²) | The only image, `grass2_D`, is a pale detail map: UV-sampled mean sRGB ≈(212,214,199), linear (0.658,0.672,0.578). The archive's `grass-detail19.png` (mean sRGB 84,88,56) was never embedded. | `baseColorFactor` [0.135,0.145,0.068] brings the UV-sampled mean to the detail map's colour. The material stays `OPAQUE`: its texture alpha averages 0.18, and treating that as coverage would make the grass vanish. |
| `top2*.001` aerial terrain (≈5.08 km²) | Aerial photo; its grass pixels (median sRGB 138,147,122) rendered near-white and left a hard seam against the repaired infield grass. | `baseColorFactor` [0.349,0.334,0.203] maps the aerial grass median onto the same grass colour. Photo variation is kept. |
| `grass_NM.001` (988 m², x 41..110, z −483..−411) | Every UV-sampled texel is RGB 0 with alpha 0, so `OPAQUE` rendered black slabs. | Added to `MASK`. Its collision surface is unchanged. |

## Audited and intentionally unchanged

- `sand_new.001` keeps its base `trap_detail`, a plausible gravel albedo (≈141,136,118).
  The spec-slot `sand_base` is a pale beige layer meant to be multiplied with it.
- `sand-bordini.001` is near-white because its atlas is the painted track-edge line.
- `jersey.*` and `tyreswall.*` have alpha ≈0 throughout. They must stay `OPAQUE`;
  the test guards this.
- The existing `MASK` rules (fences, structures, trees, hotel, `box1.001`,
  `miscalpha`) and `BLEND` rules (glass, `Transparent.*`, groove decals) were checked
  against texture alpha and kept.
- Roughness values (0.5–0.97) and metallic 0 are plausible. `KHR_materials_specular`
  and normal maps (all `scale` 0) are ignored by the converter.

## Remaining limitations

- The terrain is a single 512² aerial photo (about 4 m per texel). The loose archive has a
  4096² `top2_B.png` and 4096² `asph-old_new_B.png`, but the converter only reads
  embedded images.
- The AC detail-multiply look (grass grain, gravel × `sand_base`) is approximated by
  flat tints, not reproduced.
- `HotelConcrete`, `NewBridge.concrete` and `NewBridge.black` omit `roughnessFactor`.
  The glTF default is 1.0, but the converter uses 0.8, a minor converter-wide deviation.

## Verification

`python3 -m unittest test_silverstone_materials` checks the mappings and alpha guards.
Native Vulkan captures (headless Weston, `track_probe::rendered_track_lap` with
`F1_TEST_CAMERA` set to chase, cockpit, tv, spectator and spectator-reverse) all
reproduce the same lap: 138.61 s, worst 2.169 m, off 0, 0 contacts, 0 jumps.
