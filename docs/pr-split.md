# 2.0 and livery PR boundaries

The core branch is `feat/gen22-car-and-cockpit`, targeting `main` in PR #2.
The livery branch is `feat/gen22-liveries`, stacked on the core branch.
Merge core first. The livery PR can then be retargeted to `main`.

## Core

- Shared GEN22 geometry, player cockpit, controls and driver animation.
- Neutral stock body paint, including when loading older generated GLBs.
- Team/driver race identities and HUD colours, tyre compounds and tyre textures.
- Physics, gearing, AI racecraft, DRS, HUD and rendering/performance work.
- Asset conversion without livery authoring dependencies; stock Linux/Windows packaging.

## Livery layer

- Team artwork layouts, authentic logo preparation, native UV0 composition and review tools.
- Optional template UV authoring, seam preservation and livery-specific asset checks.
- Per-team material instances and texture selection on the same shared geometry.
- Livery tests, documentation and packaging of installed team textures.

No generated assets or source downloads are committed. Existing ignored artwork
and generated cars remain on disk. No per-team or per-driver car meshes are added.
The Git split preserves the published branch history; the final tree of PR #2
excludes the livery feature. Earlier commits remain in its history.

## Verification

Core asset conversion was run from the checkout with `tools/livery` removed,
into `~/.cache/f1-pr-split/assets/cars/gen22`. All three stock GLBs passed the
asset contract and clean Blender re-import. Runtime captures, test output and
build logs are retained under `~/.cache/f1-pr-split/`.

Technical checks do not establish photographic livery fidelity. The livery PR
preserves the work for continued visual review and does not claim artwork approval.

Core checks passed: workspace library/binary tests, Clippy with warnings denied,
Cargo formatting, opening-traffic regression, seven standalone car tests and
two package tests. The Vulkan `rendered_car_review` passed using the newly
rebuilt stock assets and produced 13 captures; the side view was visually
checked for neutral body paint. Windows/AppImage release binaries were not rebuilt.

On the stacked livery branch, the stock exporter still embeds no team body artwork.
Native UV0 team maps can be applied to the core stock GLBs without rebuilding them.
The optional template-authoring path adds UV1 for its separate template workflow.

Stacked livery checks passed: workspace tests (78 game, 44 simulation), six
focused material tests, Python livery suite (60 run, four skipped), Clippy,
formatting and package tests. Vulkan review passed with McLaren native UV0
textures applied to the exact core stock GLBs, with 13 captures. This verifies
the branch boundary and runtime binding, not reference-photo quality.
