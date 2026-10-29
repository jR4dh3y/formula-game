# User-supplied F1 2013 audio reference

Source: https://www.youtube.com/watch?v=P19YW7daEFM&t=286s

Video title: Red Bull RB9 by Mark Webber | F1 2013 Italian Grand Prix | Logitech G29 Gameplay.

Video uploader: C0llioN — https://www.youtube.com/channel/UCljV49bx4VbkYAUDMKXtfeQ

At the user's request, the engine voice samples this video's gameplay audio directly. `engine_on.wav` derives from 4:00.25–4:01.45; `engine_off.wav` derives from 4:14.2–4:16.5. Processing: mono extraction, pitch stabilization to 18,000 RPM, rumble removal, gentle level correction, cycle alignment and loop-boundary crossfade. `tools/audio/prepare_engine.py` reproduces the processing from a local copy of the 4:00–5:05 section.

Original video and game-audio rights remain with their respective owners. These samples are not covered by the separate F60/F430 CC BY-SA credits in `../v8`.
