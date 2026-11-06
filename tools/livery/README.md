# GEN22 livery pipeline (PhotoCraft + 3D projection)

Base proven on the Ferrari SF-26 (`~/livery-projects/ferrari-sf26`). Copy this folder for a new team.

## Layout
- `pc.py` – PhotoCraft MCP bridge client. Run PhotoCraft first:
  `setsid -f ~/.local/bin/photocraft --new-instance --control $PORT --control-token-file $PWD/pc.token --automation-read-root $PWD/livery --automation-write-root $PWD/livery`
  then `PC_PORT=$PORT python3 -I pc.py '[{"tool":"session_list","args":{}}]'`. Each project uses its own port (see `PC_PORT`). Paths given to PhotoCraft are relative to `livery/`.
  Bridge notes: use `layer.copyToDocument {document:0, source:1, at:[cx,cy], name}` (at = CENTRE), `document.activate`, `file.close {document:1, save:"discard"}`, `file.closeAll`; doc_place/doc_select/doc_close are unavailable. Logos must be PNG32 (palette PNGs fail to copy). Save `.pcraft` with `doc_save`, export PNG with `doc_export`.
- `preview.py` – Blender render of the car with textures swapped:
  `flock /tmp/f1-blender.lock blender -b --factory-startup --disable-autoexec --python preview.py -- assets/cars/gen22/car.glb OUTPREFIX photo,side-a,side-b,top,top-mid,front34,rear34,fwing-up,rwing,halo-in,halo-out textures_audi_main_body_stickers=PNG textures_audi_front_wing=PNG ...` (`NAME=STOCK` keeps stock).
- `livery/ref/` – UV templates (`uv_<part>.png`), island masks (`masks/`), `carbon-tile.png`, `grid128.png`, and the main-body position map `pos_main.npz` (per-texel 3D position + normal, 4096²) and `halo_mask.npy`.
- `tools/` – `zones.py` (3D-defined paint zones -> masks; edit the zone rules per team), `decal.py` (project a logo onto the car from any direction -> pre-warped PNG at the right texture spot), `trim.py` (drop stray fragments; aborts without replacing `decal_report.json` if any projection failed), `preview_compose.py`, `posmap.py` (rebuild position map for another texture), `uvframe.py`, `uvsample.py`, `overlap.py`.
- `examples/` – the Ferrari scripts: `body_v2.py`/`parts.py` (PhotoCraft layer building helpers, wings/floor/etc.), `body_fix10.py` (zones -> layers), `body_fix11.py` (decals -> layers), `decals.json` (decal specs).

## Car facts (gen22 model, metres): front = -Y, up = +Z, right side = -X. Wheelbase ~3.6 m (axles Y≈-1.8 / +1.8). Cockpit opening Y -0.98..-0.10, half-width ≤0.22; halo ring top Z≈0.88, arms X≈±0.30; nose top Z rises 0.22 (Y -2.9) → 0.66 (Y -1.2); sidepod tops Z≈0.62-0.68; engine cover to Z≈1.07.

## Lessons (do these, not 2D shapes)
1. Never paint colour zones as 2D shapes per UV island – seams tear. Define zones in 3D with `tools/zones.py` on `pos_main.npz`, pad into gutters, import each zone as an RGBA layer.
2. Never place logos flat on the UV sheet – islands are sheared/squashed. Use `tools/decal.py`: side logos `{"side":±1,"Y","Z","width"}`; anything else `{"center":[x,y,z],"dir":[toward viewer],"base":[reading direction],"width","depth","min_facing"}`. Right side reads toward the front (base -Y), left side base +Y. Then `tools/trim.py`.
3. Front wing UVs are a straight top-down projection (tex x ≈ car Y, tex y ≈ car X); flaps overlap in plan. Centre each logo in its flap's visible strip and rotate it ~17-19° to follow the flap line (see examples/parts.py).
4. Texture names in the GLB: textures_audi_{main_body_stickers,front_wing,rear_wing,floor,mirrors,winglets,cockpit_foam,front_covers,rear_covers}.
5. Judge every render against real photos strictly; check photo, side, top, halo and nose views.

## Tests

Run isolated livery helper tests without a running PhotoCraft instance:
`python3 -m unittest discover -s tools/livery -p 'test_*.py'`.


## Install into the game
`assets/cars/gen22/liveries/<team>/<part>.png` (4096), plus `lod1/` (2048) and `lod2/` (1024) copies; per-driver main bodies at `<team>/<driver>/main_body.png` (+ lod1/lod2). Driver dirs override team files.

## Publishing liveries
Liveries are not stored in git. Run `python tools/livery/pack_liveries.py --assets assets` to build a reproducible ZIP, publish it as a GitHub pre-release named `liveries-<first 16 hex of sha256>`, and pin `LIVERY_ARCHIVE_URL` / `LIVERY_ARCHIVE_SHA256` in `.github/workflows/release.yml`. CI overlays it on the base assets with `tools/package/overlay_assets.py`, replacing each team folder.
