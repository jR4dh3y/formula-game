use super::*;
use racecraft_sim::car::powertrain::ShiftRefusal;

pub(super) fn draw(hud: &Hud, style: &HudStyle, session: &Session, layout: Layout, scale: f32) {
    match layout {
        Layout::Segments => segments(hud, style, session, scale),
        Layout::Gauges => gauges(hud, style, session, scale),
    }
}

fn segments(hud: &Hud, style: &HudStyle, session: &Session, scale: f32) {
    let ink = hud.finish.colors().ink;
    let width = hud.width();
    let height = hud.height();
    let race = &session.race;
    let player = &race.entrants[session.player];
    let car = &player.car;
    let lap = (player.timing.laps_completed + 1).min(race.laps);
    let position = race.position_of(session.player);
    let current = if matches!(race.phase, Phase::Countdown { .. }) {
        0.0
    } else {
        race.time - player.timing.lap_start
    };
    // ---- Dash (bottom right): RPM LEDs, lap / gear / speed, lap time and
    // position, then pedal traces.
    let pad = 14.0 * scale;
    let gap = 10.0 * scale;
    let dash_w = 508.0 * scale;
    let led_h = 34.0 * scale;
    let main_h = 78.0 * scale;
    let lower_h = 46.0 * scale;
    let dash_h = pad + led_h + gap + main_h + gap + lower_h + 48.0 * scale;
    let px = width - dash_w - 24.0 * scale;
    let py = height - dash_h - 24.0 * scale;
    housing(hud, px, py, dash_w, dash_h, scale);
    for (x, y) in [
        (px + 7. * scale, py + 7. * scale),
        (px + dash_w - 7. * scale, py + 7. * scale),
        (px + 7. * scale, py + dash_h - 7. * scale),
        (px + dash_w - 7. * scale, py + dash_h - 7. * scale),
    ] {
        hud.disc(
            (x, y),
            3.5 * scale,
            Color::from_gray(97),
            Color::from_gray(25),
        );
        hud.line(
            (x - 1.5 * scale, y),
            (x + 1.5 * scale, y),
            scale,
            Color::from_gray(12),
        );
    }
    let inner_x = px + pad;
    let inner_w = dash_w - pad * 2.0;

    let engine = &car.spec.engine;
    let rpm_fraction = ((car.powertrain.rpm - engine.idle_rpm)
        / (engine.rev_limit - engine.idle_rpm))
        .clamp(0.0, 1.0) as f32;
    // Shift lights: green, red, then blue at the optimum change point.
    let led_y = py + pad;
    hud.fill_round(inner_x, led_y, inner_w, led_h, led_h * 0.5, WELL);
    hud.stroke_round(
        inner_x,
        led_y,
        inner_w,
        led_h,
        led_h * 0.5,
        1.0 * scale,
        WELL_RIM,
    );
    let lights = 15;
    let led_pitch = (inner_w - led_h) / (lights - 1) as f32;
    let flash = car.powertrain.limiter && (race.time * 12.0).sin() > 0.0;
    for light in 0..lights {
        let threshold = 0.55 + 0.45 * light as f32 / lights as f32;
        let lit = rpm_fraction >= threshold;
        let color = if light < 5 {
            GREEN
        } else if light < 10 {
            ACCENT
        } else {
            BLUE
        };
        let color = if flash {
            BLUE
        } else if lit {
            color
        } else {
            Color::from_rgb(80, 83, 69)
        };
        let centre = (
            inner_x + led_h * 0.5 + light as f32 * led_pitch,
            led_y + led_h * 0.5,
        );
        hud.circle(centre, 12.0 * scale, Color::BLACK);
        hud.disc(centre, 10.5 * scale, color, color.gamma_multiply(0.35));
        // Lens highlight.
        hud.circle(
            (centre.0 - 3.0 * scale, centre.1 - 3.5 * scale),
            3.0 * scale,
            rgba(1.0, 1.0, 1.0, if flash || lit { 0.45 } else { 0.08 }),
        );
    }

    let main_y = led_y + led_h + gap;
    let gear_w = 92.0 * scale;
    let side_w = (inner_w - gear_w - gap * 2.0) * 0.5;
    let gear_x = inner_x + side_w + gap;
    let speed_x = gear_x + gear_w + gap;
    // Lap / laps.
    well(hud, inner_x, main_y, side_w, main_h, scale);
    caption(
        hud,
        style,
        14.0 * scale,
        (inner_x + 10.0 * scale, main_y + 5.0 * scale),
        Align::Left,
        "LAP",
    );
    let value_y = main_y + 27.0 * scale;
    let value_size = 42.0 * scale;
    hud.segments(
        value_size,
        (inner_x + side_w - 12.0 * scale, value_y),
        Align::Right,
        ink,
        &format!("{lap}/{}", race.laps),
    );
    // Gear sits in the same recessed aperture as the other LCD readouts.
    let refusal = car.powertrain.last_refusal.map(|refusal| match refusal {
        ShiftRefusal::OverRev => "OVER-REV",
        ShiftRefusal::TooFast => "TOO FAST",
    });
    let bezel = if refusal.is_some() { AMBER } else { WELL_RIM };
    hud.fill_round(
        gear_x,
        main_y - 9. * scale,
        gear_w,
        main_h + 18. * scale,
        5.0 * scale,
        WELL,
    );
    hud.stroke_round(
        gear_x,
        main_y - 9. * scale,
        gear_w,
        main_h + 18. * scale,
        5.0 * scale,
        1.0 * scale,
        bezel,
    );
    let gear = match car.powertrain.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        gear => gear.to_string(),
    };
    let gear_size = 62.0 * scale;
    let gear_y = main_y + 8.0 * scale;
    hud.segments(
        gear_size,
        (gear_x + gear_w * 0.5, gear_y),
        Align::Center,
        ink,
        &gear,
    );
    // Speed, with engine speed as its caption.
    well(hud, speed_x, main_y, side_w, main_h, scale);
    caption(
        hud,
        style,
        14.0 * scale,
        (speed_x + 10.0 * scale, main_y + 5.0 * scale),
        Align::Left,
        "KM/H",
    );
    caption(
        hud,
        style,
        14.0 * scale,
        (speed_x + side_w - 10.0 * scale, main_y + 5.0 * scale),
        Align::Right,
        "RPM",
    );
    hud.segments(
        11. * scale,
        (speed_x + side_w - 42. * scale, main_y + 6. * scale),
        Align::Right,
        DIM,
        &format!("{:.0}", car.powertrain.rpm),
    );
    let kmh = (car.speed() * 3.6).round() as i32;
    hud.segments(
        value_size,
        (speed_x + side_w - 12.0 * scale, value_y),
        Align::Right,
        ink,
        &kmh.to_string(),
    );

    // Lower row: current lap time where a fuel readout would sit, position
    // under the speed window.
    let lower_y = main_y + main_h + 14. * scale;
    let time_w = speed_x - gap - inner_x;
    well(hud, inner_x, lower_y, time_w, lower_h, scale);
    caption(
        hud,
        style,
        14.0 * scale,
        (inner_x + 10.0 * scale, lower_y + 14.0 * scale),
        Align::Left,
        "LAP TIME",
    );
    let lower_size = 25.0 * scale;
    let lower_value_y = lower_y + 11.0 * scale;
    hud.segments(
        lower_size,
        (inner_x + time_w - 12.0 * scale, lower_value_y),
        Align::Right,
        ink,
        &format_time(current.max(0.0)),
    );
    well(hud, speed_x, lower_y, side_w, lower_h, scale);
    caption(
        hud,
        style,
        14.0 * scale,
        (speed_x + 10.0 * scale, lower_y + 14.0 * scale),
        Align::Left,
        "POS",
    );
    hud.segments(
        lower_size,
        (speed_x + side_w - 12.0 * scale, lower_value_y),
        Align::Right,
        ink,
        &format!("{position}/{}", race.entrants.len()),
    );
    if let Some(text) = refusal {
        let tag_w = hud.text_width(&style.bold, 20.0 * scale, text) + 24.0 * scale;
        let tag_x = gear_x + gear_w * 0.5 - tag_w * 0.5;
        hud.fill_round(
            tag_x,
            py - 38.0 * scale,
            tag_w,
            30.0 * scale,
            6.0 * scale,
            WELL,
        );
        hud.stroke_round(
            tag_x,
            py - 38.0 * scale,
            tag_w,
            30.0 * scale,
            6.0 * scale,
            2.0 * scale,
            AMBER,
        );
        hud.text(
            &style.bold,
            20.0 * scale,
            (gear_x + gear_w * 0.5, py - 36.0 * scale),
            Align::Center,
            AMBER,
            text,
        );
    }

    let pedal_y = lower_y + lower_h + 8. * scale;
    let pedal_w = (inner_w - 18. * scale) * 0.5;
    pedal(
        hud,
        style,
        egui::Rect::from_min_size(
            egui::pos2(inner_x, pedal_y),
            egui::vec2(pedal_w, 28. * scale),
        ),
        "BRAKE",
        session.guidance.activity.input.brake as f32,
        ACCENT,
    );
    pedal(
        hud,
        style,
        egui::Rect::from_min_size(
            egui::pos2(inner_x + pedal_w + 18. * scale, pedal_y),
            egui::vec2(pedal_w, 28. * scale),
        ),
        "THROTTLE",
        session.guidance.activity.input.throttle as f32,
        GREEN,
    );
}

fn polar(center: (f32, f32), radius: f32, degrees: f32) -> (f32, f32) {
    let angle = degrees.to_radians();
    (
        center.0 + radius * angle.cos(),
        center.1 + radius * angle.sin(),
    )
}

fn arc(
    hud: &Hud,
    center: (f32, f32),
    radius: f32,
    angles: (f32, f32),
    thickness: f32,
    color: Color,
) {
    let steps = ((angles.1 - angles.0).abs() / 4.0).ceil().max(1.0) as usize;
    let points = (0..=steps)
        .map(|i| {
            let p = polar(
                center,
                radius,
                angles.0 + (angles.1 - angles.0) * i as f32 / steps as f32,
            );
            egui::pos2(p.0, p.1)
        })
        .collect();
    hud.painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(thickness, color),
    ));
}

/// One input readout, shared by both layouts so pedals never appear twice.
fn pedal(hud: &Hud, style: &HudStyle, rect: egui::Rect, label: &str, value: f32, color: Color) {
    let scale = rect.height() / 30.;
    caption(
        hud,
        style,
        12. * scale,
        (rect.left(), rect.top()),
        Align::Left,
        label,
    );
    readout(
        hud,
        style,
        11. * scale,
        (rect.right(), rect.top() + scale),
        Align::Right,
        hud.finish.colors().ink,
        &format!("{:.0}%", value * 100.),
    );
    hud.fill_round(
        rect.left(),
        rect.bottom() - 6. * scale,
        rect.width(),
        5. * scale,
        scale,
        Color::from_rgb(3, 9, 11),
    );
    hud.fill_round(
        rect.left(),
        rect.bottom() - 6. * scale,
        rect.width() * value,
        5. * scale,
        scale,
        color,
    );
}

/// Machined bezel and matte face; every graduation maps to a simulation value.
struct Dial {
    center: (f32, f32),
    radius: f32,
    maximum: f32,
    divisions: usize,
    major_every: usize,
    redline: Option<f32>,
}

impl Dial {
    fn point(&self, fraction: f32, radius: f32) -> (f32, f32) {
        polar(self.center, self.radius * radius, 135. + fraction * 270.)
    }

    fn face(&self, hud: &Hud, style: &HudStyle, scale: f32) {
        let c = self.center;
        let r = self.radius;
        // Offset shadow, turned metal rim, gasket, then a slightly dished face.
        hud.circle(
            (c.0 + 2. * scale, c.1 + 5. * scale),
            r + 2. * scale,
            Color::from_black_alpha(90),
        );
        hud.disc(c, r, Color::from_gray(134), Color::from_gray(28));
        hud.disc(
            c,
            r - 2. * scale,
            Color::from_gray(42),
            Color::from_gray(104),
        );
        hud.circle(c, r - 4. * scale, Color::from_rgb(5, 6, 7));
        hud.disc(
            c,
            r - 7. * scale,
            Color::from_rgb(31, 33, 34),
            Color::from_rgb(13, 15, 16),
        );
        let ink = hud.finish.colors().ink;
        if let Some(redline) = self.redline {
            arc(
                hud,
                c,
                r * 0.89,
                (135. + redline * 270., 405.),
                4. * scale,
                ACCENT,
            );
        }
        for i in 0..=self.divisions {
            let fraction = i as f32 / self.divisions as f32;
            let major = i % self.major_every == 0;
            let color = if self.redline.is_some_and(|redline| fraction >= redline) {
                ACCENT
            } else {
                ink
            };
            hud.line(
                self.point(fraction, if major { 0.75 } else { 0.82 }),
                self.point(fraction, 0.89),
                if major { 1.8 } else { 0.8 } * scale,
                color,
            );
            if major {
                let p = self.point(fraction, 0.64);
                hud.text(
                    &style.regular,
                    r * 0.115,
                    (p.0, p.1 - r * 0.085),
                    Align::Center,
                    color,
                    &format!("{:.0}", fraction * self.maximum),
                );
            }
        }
    }

    fn needle(&self, hud: &Hud, value: f32, color: Color, scale: f32) {
        let theta = 135. + (value / self.maximum).clamp(0., 1.) * 270.;
        let points = [
            polar(self.center, self.radius * 0.17, theta + 180.),
            polar(self.center, 2. * scale, theta + 90.),
            polar(self.center, self.radius * 0.8, theta),
            polar(self.center, 2. * scale, theta - 90.),
        ];
        hud.painter.add(egui::Shape::convex_polygon(
            points.into_iter().map(|p| egui::pos2(p.0, p.1)).collect(),
            color,
            egui::Stroke::NONE,
        ));
        hud.disc(
            self.center,
            6. * scale,
            Color::from_gray(111),
            Color::from_gray(21),
        );
        hud.circle(self.center, 2. * scale, Color::from_gray(12));
    }
}

/// Separate instruments, arranged like a race-car binnacle rather than a UI panel.
fn gauges(hud: &Hud, style: &HudStyle, session: &Session, scale: f32) {
    let car = &session.race.entrants[session.player].car;
    let ink = hud.finish.colors().ink;
    let px = hud.width() - 510. * scale;
    let py = hud.height() - 385. * scale;
    let at = |x: f32, y: f32| (px + x * scale, py + y * scale);
    let maximum = (car.spec.engine.rev_limit / 1000.).ceil() as f32;
    let tach = Dial {
        center: at(336., 194.),
        radius: 148. * scale,
        maximum,
        divisions: (maximum * 2.) as usize,
        major_every: 4,
        redline: Some(0.92),
    };
    tach.face(hud, style, scale);
    caption(
        hud,
        style,
        13. * scale,
        at(336., 124.),
        Align::Center,
        "RACECRAFT",
    );
    caption(
        hud,
        style,
        11. * scale,
        at(336., 145.),
        Align::Center,
        "RPM × 1000",
    );
    // An odometer-like gear window is part of the face, without a coloured border.
    well(
        hud,
        px + 309. * scale,
        py + 244. * scale,
        54. * scale,
        51. * scale,
        scale,
    );
    let gear = match car.powertrain.gear {
        -1 => "R".into(),
        0 => "N".into(),
        n => n.to_string(),
    };
    hud.text(
        &style.regular,
        43. * scale,
        at(336., 240.),
        Align::Center,
        ink,
        &gear,
    );
    caption(
        hud,
        style,
        10. * scale,
        at(336., 301.),
        Align::Center,
        "GEAR",
    );
    tach.needle(hud, car.powertrain.rpm as f32 / 1000., ACCENT, scale);

    let speed = Dial {
        center: at(116., 262.),
        radius: 98. * scale,
        maximum: 360.,
        divisions: 36,
        major_every: 6,
        redline: None,
    };
    speed.face(hud, style, scale);
    caption(
        hud,
        style,
        11. * scale,
        at(116., 218.),
        Align::Center,
        "KM/H",
    );
    hud.text(
        &style.regular,
        25. * scale,
        at(116., 292.),
        Align::Center,
        ink,
        &format!("{:.0}", car.speed() * 3.6),
    );
    speed.needle(
        hud,
        car.speed() as f32 * 3.6,
        Color::from_rgb(237, 226, 193),
        scale,
    );

    let gc = at(92., 73.);
    let gr = 55. * scale;
    hud.disc(gc, gr, Color::from_gray(103), Color::from_gray(21));
    hud.circle(gc, gr - 3. * scale, WELL);
    for ring in 1..=4 {
        hud.painter.circle_stroke(
            egui::pos2(gc.0, gc.1),
            ring as f32 * 11. * scale,
            egui::Stroke::new(0.7 * scale, Color::from_gray(75)),
        );
    }
    hud.line(at(48., 73.), at(136., 73.), scale, Color::from_gray(84));
    hud.line(at(92., 29.), at(92., 117.), scale, Color::from_gray(84));
    let g = Vec2::new(-car.acceleration.y as f32, car.acceleration.x as f32) / 9.81;
    let point = g.clamp_length_max(4.) * (11. * scale);
    hud.circle((gc.0 + point.x, gc.1 + point.y), 3.5 * scale, AMBER);
    caption(
        hud,
        style,
        11. * scale,
        at(92., 132.),
        Align::Center,
        &format!("{:.1} G", g.length()),
    );

    // Pedals use small moving-coil instruments, not progress bars.
    for (x, label, value, color) in [
        (228., "THR", session.guidance.activity.input.throttle, GREEN),
        (425., "BRK", session.guidance.activity.input.brake, ACCENT),
    ] {
        let dial = Dial {
            center: at(x, 56.),
            radius: 35. * scale,
            maximum: 100.,
            divisions: 10,
            major_every: 5,
            redline: None,
        };
        dial.face(hud, style, scale);
        dial.needle(hud, value as f32 * 100., color, scale);
        caption(hud, style, 10. * scale, at(x, 94.), Align::Center, label);
    }
    if let Some(refusal) = car.powertrain.last_refusal {
        let text = match refusal {
            ShiftRefusal::OverRev => "OVER-REV",
            ShiftRefusal::TooFast => "TOO FAST",
        };
        hud.text(
            &style.bold,
            14. * scale,
            at(336., 353.),
            Align::Center,
            AMBER,
            text,
        );
    }
}

pub(super) fn tyres(hud: &Hud, style: &HudStyle, session: &Session, layout: Layout, scale: f32) {
    match layout {
        Layout::Segments => tyre_display(hud, style, session, scale),
        Layout::Gauges => tyre_dials(hud, style, session, scale),
    }
}

/// LCD tyre module matching the segmented dash: per-axle temperature and wear
/// windows, then tow and wake as LED bars.
fn tyre_display(hud: &Hud, style: &HudStyle, session: &Session, scale: f32) {
    let car = &session.race.entrants[session.player].car;
    let compound = car.setup.compound;
    let pad = 14. * scale;
    let gap = 10. * scale;
    let tyre_h = 92. * scale;
    let aero_h = 38. * scale;
    let header_h = 22. * scale;
    let panel_w = 330. * scale;
    let panel_h = pad + header_h + tyre_h + gap + aero_h + pad;
    let px = 24. * scale;
    let py = hud.height() - panel_h - 24. * scale;
    housing(hud, px, py, panel_w, panel_h, scale);
    caption(
        hud,
        style,
        13. * scale,
        (px + pad, py + pad - 2. * scale),
        Align::Left,
        &format!("{} TYRES", compound.label()),
    );
    caption(
        hud,
        style,
        13. * scale,
        (px + panel_w - pad, py + pad - 2. * scale),
        Align::Right,
        "°C / WEAR",
    );
    let inner_w = panel_w - pad * 2.;
    let cell_w = (inner_w - gap) * 0.5;
    let tyre_y = py + pad + header_h;
    for (axle, label) in [(0, "FRONT"), (1, "REAR")] {
        let tyre = car.tyres[axle];
        let band = TyreBand::of(tyre.temperature, compound);
        let color = band.color();
        let x = px + pad + axle as f32 * (cell_w + gap);
        well(hud, x, tyre_y, cell_w, tyre_h, scale);
        caption(
            hud,
            style,
            13. * scale,
            (x + 10. * scale, tyre_y + 5. * scale),
            Align::Left,
            label,
        );
        // Two tyre footprints retain the per-wheel lock and slide indication.
        for side in 0..2 {
            let wheel = &car.wheels[axle * 2 + side];
            let fx = x + cell_w - (32. - side as f32 * 13.) * scale;
            hud.fill_round(
                fx,
                tyre_y + 7. * scale,
                9. * scale,
                15. * scale,
                2. * scale,
                color,
            );
            if wheel.locked || wheel.sliding > 0.3 {
                hud.stroke_round(
                    fx,
                    tyre_y + 7. * scale,
                    9. * scale,
                    15. * scale,
                    2. * scale,
                    2. * scale,
                    WHITE,
                );
            }
        }
        hud.segments(
            34. * scale,
            (x + cell_w - 12. * scale, tyre_y + 28. * scale),
            Align::Right,
            color,
            &format!("{:.0}°C", tyre.temperature),
        );
        hud.text(
            &style.bold,
            12. * scale,
            (x + 10. * scale, tyre_y + 70. * scale),
            Align::Left,
            color,
            band.label(),
        );
        hud.segments(
            13. * scale,
            (x + cell_w - 12. * scale, tyre_y + 71. * scale),
            Align::Right,
            hud.finish.colors().ink,
            &format!("{:.0}%", tyre.wear * 100.),
        );
    }
    let aero_y = tyre_y + tyre_h + gap;
    for (slot, label, value, color) in [
        (0, "TOW", car.aero.slipstream(), BLUE),
        (1, "WAKE", car.aero.dirty_air(), AMBER),
    ] {
        let x = px + pad + slot as f32 * (cell_w + gap);
        well(hud, x, aero_y, cell_w, aero_h, scale);
        caption(
            hud,
            style,
            13. * scale,
            (x + 10. * scale, aero_y + 10. * scale),
            Align::Left,
            label,
        );
        let bars = 10;
        let bar_x = x + 56. * scale;
        let bar_w = x + cell_w - 10. * scale - bar_x;
        let pitch = bar_w / bars as f32;
        for bar in 0..bars {
            let lit = value as f32 >= (bar as f32 + 0.5) / bars as f32;
            hud.fill_rect(
                bar_x + bar as f32 * pitch,
                aero_y + 11. * scale,
                pitch - 2. * scale,
                16. * scale,
                if lit {
                    color
                } else {
                    Color::from_rgb(28, 29, 28)
                },
            );
        }
    }
}

/// Analog binnacle: tyre temperature dials and small aero meters.
fn tyre_dials(hud: &Hud, style: &HudStyle, session: &Session, scale: f32) {
    let car = &session.race.entrants[session.player].car;
    let compound = car.setup.compound;
    let base = hud.height() - 118. * scale;
    for (axle, x, label) in [(0, 98., "FRONT"), (1, 254., "REAR")] {
        let tyre = car.tyres[axle];
        let band = TyreBand::of(tyre.temperature, compound);
        let color = band.color();
        let center = (x * scale, base);
        let dial = Dial {
            center,
            radius: 69. * scale,
            maximum: 180.,
            divisions: 18,
            major_every: 6,
            redline: None,
        };
        dial.face(hud, style, scale);
        // The working window is compound-specific; colour and needle are live.
        let props = compound.properties();
        for (low, high, color) in [
            (0., props.optimal_temperature - props.window, BLUE),
            (
                props.optimal_temperature - props.window,
                props.optimal_temperature + props.window,
                GREEN,
            ),
            (
                props.optimal_temperature + props.window,
                props.optimal_temperature + 2. * props.window,
                TYRE_YELLOW,
            ),
            (props.optimal_temperature + 2. * props.window, 180., ACCENT),
        ] {
            arc(
                hud,
                center,
                62. * scale,
                (
                    135. + low as f32 / 180. * 270.,
                    135. + high as f32 / 180. * 270.,
                ),
                2. * scale,
                color,
            );
        }
        caption(
            hud,
            style,
            11. * scale,
            (center.0, base - 27. * scale),
            Align::Center,
            label,
        );
        hud.text(
            &style.regular,
            16. * scale,
            (center.0, base + 24. * scale),
            Align::Center,
            color,
            &format!("{:.0}°C", tyre.temperature),
        );
        dial.needle(hud, tyre.temperature as f32, color, scale);
        // Two tyre footprints retain the per-wheel lock and slide indication.
        for side in 0..2 {
            let wheel = &car.wheels[axle * 2 + side];
            let x = center.0 + (side as f32 * 16. - 13.) * scale;
            hud.fill_round(
                x,
                base + 53. * scale,
                10. * scale,
                17. * scale,
                3. * scale,
                color,
            );
            if wheel.locked || wheel.sliding > 0.3 {
                hud.stroke_round(
                    x,
                    base + 53. * scale,
                    10. * scale,
                    17. * scale,
                    3. * scale,
                    2. * scale,
                    WHITE,
                );
            }
        }
        hud.text(
            &style.regular,
            12. * scale,
            (center.0, base + 74. * scale),
            Align::Center,
            hud.finish.colors().ink,
            &format!("{} / {:.0}% WEAR", band.label(), tyre.wear * 100.),
        );
    }
    // Setup data is available in the pause menu; only live aero effects remain here.
    let x = 342. * scale;
    for (y, value, label, color) in [
        (base - 39. * scale, car.aero.slipstream(), "TOW", BLUE),
        (base + 38. * scale, car.aero.dirty_air(), "WAKE", AMBER),
    ] {
        let dial = Dial {
            center: (x, y),
            radius: 30. * scale,
            maximum: 100.,
            divisions: 10,
            major_every: 5,
            redline: None,
        };
        dial.face(hud, style, scale);
        dial.needle(hud, value as f32 * 100., color, scale);
        caption(
            hud,
            style,
            10. * scale,
            (x, y + 32. * scale),
            Align::Center,
            label,
        );
    }
    caption(
        hud,
        style,
        11. * scale,
        (176. * scale, base - 97. * scale),
        Align::Center,
        &format!("{}  /  TYRE °C", compound.label()),
    );
}
