# Recording-based racing V8 WAV pack (F1-inspired prototype)

All files 48,000 Hz, 16-bit stereo WAV. Unlike the earlier purely synthesized version, this version has a recorded racing V8 excerpt as the tonal core. See CREDITS_LICENSE.md before shipping your game.

## Contents
- `loops/`: 16 playable RPM loops. Each has 2-second duration at 5,500, 7,000, 9,000, 11,000, 13,000, 15,000, 17,000 and 18,000 RPM, both on-throttle and off-throttle.
- `effects/`: upshift, downshift, rev-limiter WAV.
- `V8_accel_gearshifts_demo.wav`: 10.2s acceleration with two shifts, deceleration and downshift.
- `source_F430_excerpt_processed.wav`: core excerpt (processed and resampled; NOT the full original recording).
- `manifest.json`: list and RPM metadata.
- `CREDITS_LICENSE.md`: mandatory source attribution and CC BY-SA details.

## Using in game
Run nearest two bands simultaneously and crossfade between them as RPM changes. Set playback speed to `actual RPM / sample RPM` but keep the ratio within ~0.85-1.18 when you can by selecting an adjacent band. Crossfade on/off via throttle. Smooth gain changes over at least 25 ms. When shifting, temporarily cut engine gain and play a shift one-shot. Loop wave forms may be imperfect; always crossfade on changes rather than jumping sample positions.

For authentic 2010 F1 audio, replace the tonal source with a real Formula 1 recording, e.g. `Williams-Cosworth FW32 (2010)` or `Ferrari F60 (2009)` on Wikimedia Commons (both have source-specific license requirements). A real F1 powertrain recorded across fixed RPM bands would sound more accurate than this prototype.
