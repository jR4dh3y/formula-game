# F1-Inspired Racecraft Game — Physics & Gameplay Design

**Working concept:** An accessible Formula-style racing game in which understanding racecraft matters more than mastering a difficult driving simulator.

**Design direction:** *Easy to drive, hard to race well.*


---

## 1. The Vision

The aim is **not** to make players feel like they need to learn to fly a helicopter just to complete a lap. It is to make an F1 fan feel that their racing knowledge gives them an advantage.

A knowledgeable player should be rewarded for:

- Braking late **without locking up or missing the corner**.
- Choosing a good entry, apex, and exit rather than just entering corners at maximum speed.
- Following another car closely while managing the effect of dirty air.
- Using a slipstream effectively to create overtaking opportunities.
- Fighting wheel-to-wheel, defending, switching back, and anticipating opponents.
- Picking the right gear, managing tyre temperatures, and selecting an appropriate aero setup.

A new player should be able to drive confidently within minutes. A strong player should still be able to gain tenths, make better decisions, and win battles through racecraft after many hours.

**The guiding distinction:** This is a **racecraft game**, not a hardcore vehicle simulator. The challenge should primarily be *racing opponents*, not *keeping the car under control*.

## 2. From Full Simulation to Simcade

We initially considered the systems required for a very realistic F1 simulator. Those remain useful references, but the intended game should approximate many of them rather than simulate every mechanical detail.

### Full-simulation systems considered

| System | What a realistic simulator would model | Role in this game |
|---|---|---|
| Tyres | Slip angle, longitudinal slip, combined grip, load sensitivity, heating, wear | **Core, simplified** |
| Aerodynamics | Downforce, drag, aero balance, floor/ride-height sensitivity | **Core, simplified** |
| Suspension | Springs, dampers, anti-roll bars, wheel travel, camber | Basic stable vehicle handling; detailed geometry optional |
| Vehicle dynamics | Mass, centre of gravity, yaw, pitch, roll, weight transfer | **Core, simplified** |
| Powertrain | Torque curve, gearbox, differential, engine braking | **Core, simplified** |
| Braking | Brake torque, lock-up, brake bias, trail braking | **Core** |
| Track surface | Elevation, banking, bumps, kerbs, friction | **Core to believable tracks** |
| Hybrid energy | Battery, deployment, regeneration | Optional, especially for modern-era rules |
| Tyre degradation | Wear, overheating, graining, flat spots | Basic wear recommended; advanced detail optional |
| Other-car aerodynamics | Dirty air and slipstream | **Core** |
| Weather | Rain, standing water, track temperature, wind | Later feature |
| Damage | Wings, punctures, suspension, floor | Later feature |
| Fuel | Fuel consumption, changing car mass | Later feature |
| Advanced aero | Floor stall, detailed turbulent wake and aero maps | Not required for initial release |

**What to avoid initially:** Full CFD, highly detailed tyre models, chassis flex, complex suspension geometry, precise brake-by-wire simulation, and any system that adds development cost without improving racing decisions.

## 3. The Core Physics & Gameplay Systems

### 3.1 Manual gearing

Manual shifting should provide engagement and some performance advantage without being punishing.

- Eight forward gears for an F1-inspired gearbox (if matching modern F1-style cars).
- RPM, a usable powerband, a rev limiter, and sensible gear ratios.
- Shifting too early or too late loses acceleration.
- The correct gear improves corner exit and can influence engine braking on entry.
- Clutch control is automatic; the player selects the gear.
- Invalid or poorly timed shifts should cause a small, readable penalty rather than catastrophic failure.

### 3.2 Tyre temperature and grip

Tyre temperatures should affect driving in ways the player can understand and react to.

- Tyres warm from cornering, heavy braking, acceleration/wheelspin, and sliding.
- Tyres cool on straights or during less demanding driving.
- Each tyre compound has an approximate optimal temperature window.
- Cold tyres have reduced grip. Tyres near their window perform best. Overheated tyres gradually lose grip.
- Sustained dirty-air running, excessive sliding, or aggressive driving can raise temperature.
- Consider front/rear temperatures for the first implementation; individual-wheel temperatures can come later.

Use a clear temperature/grip curve rather than a complicated thermal simulation. The goal is for players to recognize, for example, “my fronts are overheating because I keep following too closely through fast corners.”

**Recommended addition: tyre wear.** Heat, sliding, and distance can increase wear. Wear gradually reduces performance and provides a strategic reason to drive smoothly, without forcing the player to become a race engineer.

### 3.3 Tyre grip and car handling

The car should have a finite grip budget shared between braking, accelerating, and turning.

- A tyre cannot provide maximum braking and maximum cornering force simultaneously.
- Excess speed or braking while turning causes understeer, a slide, or a missed apex.
- Tyre temperature and wear affect the available grip.
- Aerodynamic downforce improves high-speed grip.
- The car should communicate the limit clearly and allow recoverable mistakes.

A simplified combined-grip model is adequate. Full Pacejka-style modelling can be explored later but is not necessary to meet the design goal.

### 3.4 Aerodynamic setup

Allow adjustment of **front and rear wing levels** before the race, with clear trade-offs:

- Higher wing levels: more downforce, better high-speed cornering, more drag, lower top-speed potential.
- Lower wing levels: less drag, better straight-line performance, weaker fast-corner grip.
- More front-relative-to-rear wing: sharper turn-in, potentially more oversteer.
- More rear-relative-to-front wing: more stability, potentially more understeer.

The setup should matter by track: a low-drag configuration might help on a long straight, while a higher-downforce configuration helps around a twistier circuit.

The player should understand the trade-off at a glance. Do not require obscure engineering knowledge to make useful choices.

### 3.5 Slipstream

Following a car along a straight reduces aerodynamic drag, providing a speed advantage and helping set up overtakes.

- Effect grows as the gap closes, within sensible limits.
- Strongest when behind the leading car's aerodynamic wake and broadly aligned with it.
- Reduced when moving far off-line, too far behind, or out of the wake.
- The increase in speed should be noticeable but should not make every overtake automatic.

### 3.6 Dirty air

Following another car closely through aerodynamic corners reduces available downforce, making the car less planted.

- More relevant at medium/high speed than in very slow corners.
- Stronger at close distances and when directly behind the car ahead.
- The follower may have to lift slightly or adjust its line to maintain grip.
- Repeated sliding can raise front tyre temperatures and affect subsequent corners.
- Moving partially out of the wake should recover some clean air.

**Critical balancing rule:** Dirty air must create meaningful choices without making close following frustrating. In this game, it should be a *manageable penalty*, not a punishment for trying to race.

### 3.7 Braking and lock-ups

Braking is one of the most important skill mechanics.

- Players gain time by braking later **only if they still make the apex and get a strong exit**.
- Higher speeds can allow stronger braking because of downforce; as speed drops, grip available from aero falls.
- Braking and steering compete for the same grip budget.
- Braking too hard or too late can cause a short lock-up, wider line, overshoot, or reduced exit speed.
- Trail braking (gradually releasing brake while beginning to turn) should be rewarding, but not mandatory for basic control.
- A single lock-up should **not** ruin an entire race. Small errors should be survivable.

**Design test:** A driver who brakes later but misses the apex should lose time to a driver who brakes slightly earlier, rotates the car cleanly, and accelerates sooner.

## 4. The Car-Fighting Experience

Players should be rewarded through the race itself (time gained, a better exit, an overtaking position), **not primarily through artificial score pop-ups**.

| Racecraft decision | Reward when executed well | Risk or trade-off |
|---|---|---|
| Brake later | Gain distance into a corner | Lock-up, overshoot, poor exit |
| Trail brake | Rotate the car and reach the apex efficiently | Instability or excess front-tyre load |
| Use slipstream | Close distance and launch an attack | Less reaction time; must plan next move |
| Follow closely | Remain in striking range | Dirty air; temperature buildup |
| Take an alternative line | Better exit, switchback or clean air | Sacrifices entry speed or position |
| Defend inside | Protect the apex | May leave the defender vulnerable on exit |
| Attack outside | Preserve momentum and create a better exit | Longer route and less margin |
| Manage tyre temperature | Consistent grip over laps | Driving too conservatively loses immediate time |
| Choose the right gear | Stronger exit acceleration | Wrong timing costs momentum |
| Tune aero | Gain advantage in chosen track sections | Lose performance elsewhere |

### Example: an actual battle

1. **Follow:** Stay within roughly half a second through a corner while managing front grip.
2. **Prepare:** Choose a line that preserves the exit, rather than simply sticking to the leader's rear wing.
3. **Close:** Use the slipstream on the next straight.
4. **Commit:** Decide whether to attack on the inside or outside.
5. **Brake:** Brake late, but still make the corner. A desperate dive that misses the apex should fail naturally.
6. **Counterattack:** If the attacking car overshoots, the defender can attempt a switchback.
7. **Defend:** After passing, protect the next corner while managing exit speed and tyre condition.

That **Follow → Pressure → Attack → Defend** loop is the center of the game.

### Make alternate lines viable

The game must permit organic side-by-side battles rather than scripted passing animations.

- The inside line should offer control of the apex but may compromise the exit.
- The outside line can sometimes preserve momentum and allow a stronger exit.
- Switchbacks should be possible when someone turns in too deep or brakes too late.
- Small wheel-to-wheel contact should be survivable; high-speed ramming should have meaningful consequences.
- Opponent cars should occupy space, respond to threats, and leave reasonable racing room.

## 5. Arcade Assistance: Help With Control, Not With Racecraft

The game should be accessible on a keyboard or controller, without eliminating skill expression.

| Assist handled by the game | Decision still made by the player |
|---|---|
| Steering smoothing | Where, when, and how much to turn |
| Mild slide recovery | How fast to approach and how to take a corner |
| Partial anti-lock assistance | Braking point, pressure and entry speed |
| Low-speed traction smoothing | When to accelerate out of a corner |
| Stability assistance over kerbs | Whether to attack the kerb and at what speed |
| Automatic clutch | Which gear to select |

**Do not make the assists perfect.** If maximum braking always gives the shortest stopping distance, or excessive steering always magically follows the ideal line, the player loses the chance to develop skill.

**Do not make mistakes disproportionately costly.** A brief front lock-up, slight slide, or suboptimal gear should cost time, not necessarily end the race.

A sensible target is that new players can complete a clean lap quickly, while experienced players continue to find tenths through braking, corner exits, gears, racing lines, setup, and positioning.

## 6. Track Physics Matter

A visually accurate track is not enough if its drivable geometry is wrong.

At minimum, the track should provide:

- A smooth, accurate collision/driving surface.
- Meaningful elevation changes and banking.
- Correct corner radii and track widths.
- Kerbs that affect handling but do not toss the vehicle unrealistically.
- Surface grip values and boundaries/walls.
- Reliable racing-line and track-position data for AI and gameplay systems.

Different circuits should feel different: Monaco's tight walls and bumps, Spa's elevation and high-speed corners, and Baku's long straight and street-circuit braking zones should each change how players race.

Wet surfaces, variable track rubber, detailed kerb impacts, and localized grip changes can be added later.

## 7. A Practical Technical Model

Use a simple rigid-body vehicle model with four wheel/contact points or equivalent stable handling model, running independently of rendering. Prefer a **fixed physics timestep**, then interpolate movement for display. A physics update rate in the approximate **200–500 Hz** range is a possible starting point for experimentation, not a requirement; choose it based on measured stability and performance.

### Useful simplified physics relationships

**Aerodynamic downforce (magnitude):**

```text
Downforce = 0.5 * air_density * speed^2 * downforce_coefficient * reference_area
```

**Aerodynamic drag:**

```text
Drag = 0.5 * air_density * speed^2 * drag_coefficient * reference_area
```

**First-order tyre grip limit:**

```text
GripLimit ≈ friction_coefficient * vertical_tyre_load
```

For gameplay, make friction coefficient depend on temperature, compound, wear, and road surface. Model longitudinal/lateral force sharing so that full braking plus full steering exceeds the available grip.

A conceptual game-loop calculation could look like:

```text
for each fixed physics tick:
    read player input (steer, throttle, brake, shift)

    update RPM, gear and driving torque
    update proximity and relative position to nearby cars
    calculate slipstream drag reduction
    calculate dirty-air downforce reduction

    calculate wing-based drag and downforce
    estimate vertical load on each tyre
    compute available grip from temperature, wear, load and surface
    allocate tyre grip between braking, acceleration and cornering

    apply controllable slip/lock-up and mild recovery assists
    integrate vehicle motion and collision response
    update tyre temperature and wear
```

This is a **design sketch**, not production-ready physics code. The precise force model should be tuned for fun, stability, predictable behaviour, and believable racing trade-offs.

### Example tuning values — not real F1 measurements

- **Aero setup:** Two sliders (front wing, rear wing), perhaps `0–10` each.
- **Close-follow slipstream:** An illustrative reduction of several percent in drag, fading with distance and lateral offset.
- **Dirty air:** A modest illustrative reduction in cornering aero grip, fading with distance and speed.
- **Tyre model:** An optimal temperature band with smooth performance falloff outside it.
- **Lock-up:** Short loss of lateral grip and a small temperature/wear penalty, recoverable when the player releases brake pressure.

*All percentages, curves, and thresholds should be play-tested. The earlier UI sliders and example 10–12% maximum effects were explanatory prototypes, not established physics targets.*

## 8. Game AI Is Just as Important as Physics

If the purpose is great racing battles, opponent AI needs to be capable of *racing*, not just following a centerline.

AI should:

- Use reasonable braking points and occasionally make small mistakes.
- Follow closely and use slipstream to attack.
- Account for dirty air when choosing a line.
- Defend an inside line without weaving unpredictably.
- Know when a move is unrealistic and when to back out.
- Leave room when cars are genuinely alongside.
- Attempt switchbacks or prioritize exits when appropriate.
- Handle differences in tyre state and car setup.

A convincing battle against another driver or AI opponent is the primary way to demonstrate the value of the physics systems.
