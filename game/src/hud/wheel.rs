//! The steering wheel's own LCD, painted into the texture on the player's
//! cockpit wheel. It shares the HUD's segmented theme so the wheel and the
//! screen dash read as one instrument set.

use super::*;
use racecraft_sim::car::powertrain::ShiftRefusal;

/// Logical size of the wheel screen; matches the LCD quad's 1.72:1 aspect.
pub const SIZE: (u32, u32) = (768, 448);
/// Height of the strip used by the slim pod on wheels without their own LCD.
pub const COMPACT_HEIGHT: f32 = 156.;

const PAD: f32 = 14.;
const GAP: f32 = 10.;
const CELL_H: f32 = 124.;

fn cell(hud: &Hud, style: &HudStyle, x: f32, y: f32, w: f32, label: &str) {
    well(hud, x, y, w, CELL_H, 1.);
    hud.text_mid(
        &style.bold,
        22.,
        (x + 14., y + 22.),
        Align::Left,
        hud.finish.colors().muted,
        label,
    );
}

fn value(hud: &Hud, x: f32, y: f32, w: f32, height: f32, color: Color, text: &str) {
    hud.segments(
        height,
        (x + w - 14., y + 44. + (66. - height) * 0.5),
        Align::Right,
        color,
        text,
    );
}

pub fn draw(hud: &Hud, style: &HudStyle, session: &Session) {
    let colors = hud.finish.colors();
    let race = &session.race;
    let player = &race.entrants[session.player];
    let car = &player.car;
    hud.fill_rect(0., 0., hud.width(), hud.height(), Color::from_rgb(4, 6, 7));

    let side_w = 248.;
    let left = PAD;
    let right = hud.width() - PAD - side_w;
    let rows = [PAD, PAD + CELL_H + GAP, PAD + (CELL_H + GAP) * 2.];

    // Left column: race state.
    cell(hud, style, left, rows[0], side_w, "DELTA");
    let (delta, delta_color) = match session.delta.current {
        Some(delta) => (
            format!("{delta:+.2}"),
            if delta <= 0. { GREEN } else { ACCENT },
        ),
        None => ("-.--".into(), colors.muted),
    };
    value(hud, left, rows[0], side_w, 62., delta_color, &delta);
    cell(hud, style, left, rows[1], side_w, "LAP");
    let lap = (player.timing.laps_completed + 1).min(race.laps);
    value(
        hud,
        left,
        rows[1],
        side_w,
        62.,
        colors.ink,
        &format!("{lap}/{}", race.laps),
    );
    cell(hud, style, left, rows[2], side_w, "POS");
    value(
        hud,
        left,
        rows[2],
        side_w,
        62.,
        colors.ink,
        &format!(
            "{}/{}",
            race.position_of(session.player),
            race.entrants.len()
        ),
    );

    // Right column: car state.
    cell(hud, style, right, rows[0], side_w, "KM/H");
    value(
        hud,
        right,
        rows[0],
        side_w,
        62.,
        colors.ink,
        &format!("{:.0}", car.speed() * 3.6),
    );
    cell(hud, style, right, rows[1], side_w, "LAP TIME");
    let current = if matches!(race.phase, Phase::Countdown { .. }) {
        0.
    } else {
        race.time - player.timing.lap_start
    };
    value(
        hud,
        right,
        rows[1],
        side_w,
        40.,
        colors.ink,
        &format_time(current.max(0.)),
    );
    cell(hud, style, right, rows[2], side_w, "TYRES °C");
    for (axle, label) in [(0, "F"), (1, "R")] {
        let tyre = car.tyres[axle];
        let band = TyreBand::of(tyre.temperature, car.setup.compound);
        let mid = rows[2] + 62. + axle as f32 * 38.;
        hud.text_mid(
            &style.bold,
            22.,
            (right + 14., mid),
            Align::Left,
            colors.muted,
            label,
        );
        for side in 0..2 {
            let wheel = &car.wheels[axle * 2 + side];
            let x = right + 46. + side as f32 * 22.;
            hud.fill_round(x, mid - 13., 16., 26., 3., band.color());
            if wheel.locked || wheel.sliding > 0.3 {
                hud.stroke_round(x, mid - 13., 16., 26., 3., 3., WHITE);
            }
        }
        hud.segments(
            30.,
            (right + side_w - 14., mid - 15.),
            Align::Right,
            band.color(),
            &format!("{:.0}", tyre.temperature),
        );
    }

    // Gear: the biggest thing on the wheel, and the shift light's echo.
    let gear_x = left + side_w + GAP;
    let gear_w = right - GAP - gear_x;
    let gear_h = CELL_H * 3. + GAP * 2.;
    well(hud, gear_x, PAD, gear_w, gear_h, 1.);
    let engine = &car.spec.engine;
    let rpm_fraction = ((car.powertrain.rpm - engine.idle_rpm)
        / (engine.rev_limit - engine.idle_rpm))
        .clamp(0., 1.) as f32;
    let flash = car.powertrain.limiter && (race.time * 12.).sin() > 0.;
    let gear_color = if flash || (rpm_fraction >= 0.92 && !car.powertrain.limiter) {
        BLUE
    } else {
        colors.ink
    };
    let gear = match car.powertrain.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        n => n.to_string(),
    };
    hud.segments(
        250.,
        (gear_x + gear_w * 0.5 + 10., PAD + 40.),
        Align::Center,
        gear_color,
        &gear,
    );
    hud.text_mid(
        &style.bold,
        20.,
        (gear_x + 14., PAD + gear_h - 30.),
        Align::Left,
        colors.muted,
        "RPM",
    );
    hud.segments(
        30.,
        (gear_x + gear_w - 14., PAD + gear_h - 45.),
        Align::Right,
        colors.ink,
        &format!("{:.0}", car.powertrain.rpm),
    );
    if let Some(refusal) = car.powertrain.last_refusal {
        let text = match refusal {
            ShiftRefusal::OverRev => "OVER-REV",
            ShiftRefusal::TooFast => "TOO FAST",
        };
        hud.fill_round(gear_x + 8., PAD + 8., gear_w - 16., 44., 4., AMBER);
        hud.text_mid(
            &style.bold,
            30.,
            (gear_x + gear_w * 0.5, PAD + 30.),
            Align::Center,
            Color::from_rgb(10, 10, 10),
            text,
        );
    }

    // Pedal traces along the bottom edge.
    let bar_y = hud.height() - PAD - 14.;
    let bar_w = (hud.width() - PAD * 2. - GAP) * 0.5;
    let input = session.guidance.activity.input;
    for (x, value, color) in [
        (PAD, input.brake, ACCENT),
        (PAD + bar_w + GAP, input.throttle, GREEN),
    ] {
        hud.fill_round(x, bar_y, bar_w, 14., 3., Color::from_rgb(16, 19, 20));
        hud.fill_round(x, bar_y, bar_w * value.clamp(0., 1.) as f32, 14., 3., color);
    }
}

/// Shared rev-light rule for physical LEDs and on-screen lamps: green, red,
/// then blue towards the change point, blue flashing on the limiter, and the
/// start gantry's reds during the countdown.
pub fn rev_light(session: &Session, index: usize, count: usize) -> Option<Color> {
    let race = &session.race;
    let car = &race.entrants[session.player].car;
    if let Phase::Countdown { lights, .. } = race.phase {
        return (index < lights as usize * count / 5).then_some(ACCENT);
    }
    if car.powertrain.limiter {
        return ((race.time * 12.).sin() > 0.).then_some(BLUE);
    }
    let engine = &car.spec.engine;
    let rpm = ((car.powertrain.rpm - engine.idle_rpm) / (engine.rev_limit - engine.idle_rpm))
        .clamp(0., 1.) as f32;
    (rpm >= 0.55 + 0.45 * index as f32 / count as f32).then_some(match index * 3 / count {
        0 => GREEN,
        1 => ACCENT,
        _ => BLUE,
    })
}

/// Slim pod display for wheels modelled without an LCD: rev lights across
/// the top, then speed, gear and delta.
pub fn draw_compact(hud: &Hud, style: &HudStyle, session: &Session) {
    let colors = hud.finish.colors();
    let car = &session.race.entrants[session.player].car;
    let width = hud.width();
    hud.fill_rect(0., 0., width, COMPACT_HEIGHT, Color::from_rgb(4, 6, 7));
    let lamps = 15;
    let pitch = (width - 80.) / (lamps - 1) as f32;
    for index in 0..lamps {
        let centre = (40. + index as f32 * pitch, 18.);
        hud.circle(centre, 10., Color::BLACK);
        hud.circle(
            centre,
            8.,
            rev_light(session, index, lamps).unwrap_or(Color::from_rgb(28, 30, 28)),
        );
    }
    let (y, h) = (38., COMPACT_HEIGHT - 46.);
    let side_w = 252.;
    let gear_x = 10. + side_w + 10.;
    let gear_w = width - 2. * (side_w + 20.);
    for (x, label) in [(10., "KM/H"), (width - 10. - side_w, "DELTA")] {
        well(hud, x, y, side_w, h, 1.);
        hud.text_mid(
            &style.bold,
            20.,
            (x + 12., y + 18.),
            Align::Left,
            colors.muted,
            label,
        );
    }
    hud.segments(
        58.,
        (10. + side_w - 12., y + 40.),
        Align::Right,
        colors.ink,
        &format!("{:.0}", car.speed() * 3.6),
    );
    let (delta, delta_color) = match session.delta.current {
        Some(delta) => (
            format!("{delta:+.2}"),
            if delta <= 0. { GREEN } else { ACCENT },
        ),
        None => ("-.--".into(), colors.muted),
    };
    hud.segments(
        58.,
        (width - 22., y + 40.),
        Align::Right,
        delta_color,
        &delta,
    );
    well(hud, gear_x, y, gear_w, h, 1.);
    let gear = match car.powertrain.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        n => n.to_string(),
    };
    let shifting = rev_light(session, 14, 15).is_some()
        && !matches!(session.race.phase, Phase::Countdown { .. });
    hud.segments(
        h - 16.,
        (gear_x + gear_w * 0.5 + 6., y + 8.),
        Align::Center,
        if shifting { BLUE } else { colors.ink },
        &gear,
    );
}
