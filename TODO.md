# To do

## Steering wheel and DRS

- **Thumb press.** Bake a driver thumb pose and trigger it when the player
  presses DRS, using the steering and shift-finger animation path in
  `tools/car/driver_steer.py`.
- **Button travel.** Make the DRS button cap a separate movable part and animate
  it pushing in when DRS is pressed.
- **Wheel screen.** Show DRS on the steering-wheel screen in
  `game/src/wheel_dash.rs`, matching the HUD state (available and open).
- **Button and dial labels.** Restore the labels/legends on the steering-wheel
  buttons and dials that were stripped during conversion; keep the screen's
  own display content.

## Testing cars

- Add a vanilla testing mode that runs every car without any livery.

## Track and cameras

- Make Eau Rouge flat out.
- Fix the TV camera position.
- Fix the cockpit view at full steering lock so the camera does not end up
  inside the driver's arm.

## Liveries

- Monoposto sheet mapping still stretches (p50 about 2x on the body, more on
  the mirrors), and some sponsor lettering crosses the rear-wing endplate
  notch. See `docs/gen22-mapping-evidence.md`.
- Convert the remaining livery PNGs once the mapping is approved.
- Per-team paint finish (metallic silver for Mercedes, matte for Red Bull).

## Other

- Car selection menu redesign.
- Measure uncapped frame rate on a real display (headless Weston caps at 60).
