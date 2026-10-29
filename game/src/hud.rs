//! Race HUD drawn through egui in Bevy.

use crate::camera::Mode;
use crate::canvas::{rgba, Align, Hud};
use crate::Session;
use bevy::prelude::*;
use bevy_egui::{
    egui::{self, Color32 as Color, FontFamily},
    EguiContexts,
};
use racecraft_sim::car::powertrain::ShiftRefusal;
use racecraft_sim::race::{Phase, RaceEvent};
use racecraft_sim::Track;

#[derive(Resource)]
pub struct HudStyle {
    regular: FontFamily,
    bold: FontFamily,
}

const PANEL: Color = Color::from_rgba_premultiplied(4, 4, 6, 184);
const WHITE: Color = Color::WHITE;
const DIM: Color = Color::from_rgb(184, 191, 204);
const ACCENT: Color = Color::from_rgb(237, 33, 43);
const PURPLE: Color = Color::from_rgb(184, 92, 255);
const GREEN: Color = Color::from_rgb(64, 230, 115);
const AMBER: Color = Color::from_rgb(255, 184, 38);
const BLUE: Color = Color::from_rgb(77, 153, 255);

pub fn setup(mut commands: Commands) {
    commands.insert_resource(HudStyle {
        regular: FontFamily::Name("regular".into()),
        bold: FontFamily::Name("bold".into()),
    });
}

pub fn update(
    mut contexts: EguiContexts,
    style: Res<HudStyle>,
    mut session: ResMut<Session>,
    time: Res<Time>,
    mut initialized: Local<bool>,
) -> Result {
    let context = contexts.ctx_mut()?;
    if !*initialized {
        let mut fonts = egui::FontDefinitions::empty();
        for (name, data) in [
            (
                "regular",
                include_bytes!("../fonts/TitilliumWeb-SemiBold.ttf").as_slice(),
            ),
            (
                "bold",
                include_bytes!("../fonts/TitilliumWeb-Bold.ttf").as_slice(),
            ),
        ] {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_static(data).into());
            fonts
                .families
                .insert(FontFamily::Name(name.into()), vec![name.into()]);
        }
        // egui's defaults can still refer to these two built-in family names.
        fonts
            .families
            .insert(FontFamily::Proportional, vec!["regular".into()]);
        fonts
            .families
            .insert(FontFamily::Monospace, vec!["regular".into()]);
        context.set_fonts(fonts);
        *initialized = true;
        return Ok(());
    }
    let mut hud = Hud {
        painter: context.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("race-hud"),
        )),
        size: context.content_rect().size(),
    };
    if session.ready {
        draw(&mut hud, &style, &mut session, time.delta_secs());
    } else {
        hud.text(
            &style.bold,
            32.0,
            (hud.width() * 0.5, hud.height() * 0.45),
            Align::Center,
            WHITE,
            "LOADING TRACK AND CARS",
        );
    }
    Ok(())
}

/// Track outline scaled into a unit box, for the minimap.
pub struct Minimap {
    points: Vec<(f32, f32)>,
    origin: (f64, f64),
    span: f64,
    margin: (f64, f64),
}

impl Minimap {
    pub fn new(track: &Track) -> Self {
        let (mut min_x, mut min_z, mut max_x, mut max_z) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for station in &track.stations {
            min_x = min_x.min(station.position.x);
            min_z = min_z.min(station.position.z);
            max_x = max_x.max(station.position.x);
            max_z = max_z.max(station.position.z);
        }
        let span = (max_x - min_x).max(max_z - min_z).max(1.0);
        let mut minimap = Self {
            points: Vec::new(),
            origin: (min_x, min_z),
            span,
            margin: (
                (1.0 - (max_x - min_x) / span) * 0.5,
                (1.0 - (max_z - min_z) / span) * 0.5,
            ),
        };
        minimap.points = track
            .stations
            .iter()
            .step_by(6)
            .map(|station| minimap.project(station.position.x, station.position.z))
            .collect();
        minimap
    }

    fn project(&self, x: f64, z: f64) -> (f32, f32) {
        (
            ((x - self.origin.0) / self.span + self.margin.0) as f32,
            ((z - self.origin.1) / self.span + self.margin.1) as f32,
        )
    }
}

/// Short-lived banner messages.
#[derive(Default)]
pub struct Messages {
    active: Vec<(String, Color, f32)>,
    seen_events: usize,
}

impl Messages {
    fn push(&mut self, text: impl Into<String>, color: Color, seconds: f32) {
        let text = text.into();
        self.active.retain(|(existing, _, _)| *existing != text);
        self.active.push((text, color, seconds));
        if self.active.len() > 3 {
            self.active.remove(0);
        }
    }
}

pub fn format_time(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    let rest = seconds - minutes * 60.0;
    format!("{}:{:06.3}", minutes as u32, rest)
}

fn follow_events(session: &mut Session) {
    let player = session.player;
    let events = &session.race.events[session.messages.seen_events..];
    let mut pending = Vec::new();
    for event in events {
        match *event {
            RaceEvent::LightsOut => pending.push(("LIGHTS OUT", GREEN, 2.0)),
            RaceEvent::LapCompleted { entrant, time, .. } if entrant == player => {
                let best = session.race.entrants[player].timing.best_lap == Some(time);
                pending.push((
                    if best {
                        "PERSONAL BEST"
                    } else {
                        "LAP COMPLETE"
                    },
                    if best { PURPLE } else { WHITE },
                    2.5,
                ));
                let text = format_time(time);
                session.messages.push(text, WHITE, 2.5);
            }
            RaceEvent::Overtake { entrant, passed } if entrant == player => {
                let name = session.race.entrants[passed].name.clone();
                session
                    .messages
                    .push(format!("PASSED {}", name.to_uppercase()), GREEN, 2.0);
            }
            RaceEvent::Overtake { entrant, passed } if passed == player => {
                let name = session.race.entrants[entrant].name.clone();
                session
                    .messages
                    .push(format!("PASSED BY {}", name.to_uppercase()), AMBER, 2.0);
            }
            RaceEvent::Finished { entrant, position } if entrant == player => {
                session
                    .messages
                    .push(format!("FINISHED P{position}"), WHITE, 6.0);
            }
            RaceEvent::Contact { a, b, impulse }
                if (a == player || b == Some(player)) && impulse > 6_000.0 =>
            {
                pending.push(("CONTACT", ACCENT, 1.2));
            }
            _ => {}
        }
    }
    session.messages.seen_events = session.race.events.len();
    for (text, color, seconds) in pending {
        session.messages.push(text, color, seconds);
    }
}

pub fn draw(hud: &mut Hud, style: &HudStyle, session: &mut Session, dt: f32) {
    if hud.width() == 0.0 {
        return;
    }
    follow_events(session);
    if session.race.phase == Phase::Finished {
        session.messages.active.clear();
    }
    for message in &mut session.messages.active {
        message.2 -= dt;
    }
    session.messages.active.retain(|message| message.2 > 0.0);

    let scale = hud.height() / 1080.0;
    let width = hud.width();
    let height = hud.height();
    let race = &session.race;
    let player = &race.entrants[session.player];
    let car = &player.car;

    // ---- Timing tower (top left)
    let tower_x = 24.0 * scale;
    let mut y = 24.0 * scale;
    let row = 30.0 * scale;
    hud.fill_rect(tower_x, y, 300.0 * scale, row * 1.5, ACCENT);
    let lap = (player.timing.laps_completed + 1).min(race.laps);
    hud.text(
        &style.bold,
        26.0 * scale,
        (tower_x + 12.0 * scale, y + 6.0 * scale),
        Align::Left,
        WHITE,
        &format!("LAP {lap}/{}", race.laps),
    );
    let position = race.position_of(session.player);
    hud.text(
        &style.bold,
        26.0 * scale,
        (tower_x + 288.0 * scale, y + 6.0 * scale),
        Align::Right,
        WHITE,
        &format!("P{position}/{}", race.entrants.len()),
    );
    y += row * 1.5;
    let leader_distance = race.entrants[race.order()[0]].timing.distance;
    for (rank, &index) in race.order().iter().enumerate() {
        let entrant = &race.entrants[index];
        let mine = index == session.player;
        hud.fill_rect(
            tower_x,
            y,
            300.0 * scale,
            row,
            if mine {
                rgba(0.25, 0.27, 0.32, 0.85)
            } else {
                PANEL
            },
        );
        hud.text(
            &style.bold,
            20.0 * scale,
            (tower_x + 10.0 * scale, y + 3.0 * scale),
            Align::Left,
            DIM,
            &format!("{:>2}", rank + 1),
        );
        let team = if entrant.car.spec.name == "RB22" {
            rgba(0.2, 0.3, 0.9, 1.0)
        } else {
            rgba(0.9, 0.75, 0.1, 1.0)
        };
        hud.fill_rect(
            tower_x + 40.0 * scale,
            y + 6.0 * scale,
            4.0 * scale,
            row - 12.0 * scale,
            team,
        );
        hud.text(
            &style.regular,
            20.0 * scale,
            (tower_x + 52.0 * scale, y + 3.0 * scale),
            Align::Left,
            if mine { WHITE } else { DIM },
            &entrant.name.to_uppercase(),
        );
        let gap = if rank == 0 {
            "LEADER".to_string()
        } else if let Some(finish) = entrant.timing.finish_time {
            format!(
                "{:+.1}",
                finish
                    - race.entrants[race.order()[0]]
                        .timing
                        .finish_time
                        .unwrap_or(finish)
            )
        } else {
            let metres = leader_distance - entrant.timing.distance;
            if metres > race.track.length {
                format!("+{} LAP", (metres / race.track.length) as u32)
            } else {
                format!("+{:.1}", metres / car.speed().max(30.0))
            }
        };
        hud.text(
            &style.regular,
            18.0 * scale,
            (tower_x + 290.0 * scale, y + 4.0 * scale),
            Align::Right,
            DIM,
            &gap,
        );
        y += row;
    }
    // Lap times under the tower.
    y += 10.0 * scale;
    hud.fill_rect(tower_x, y, 300.0 * scale, row * 3.0, PANEL);
    let current = if matches!(race.phase, Phase::Countdown { .. }) {
        0.0
    } else {
        race.time - player.timing.lap_start
    };
    let lines = [
        ("TIME", format_time(current.max(0.0)), WHITE),
        (
            "LAST",
            player
                .timing
                .last_lap
                .map(format_time)
                .unwrap_or_else(|| "-:--.---".into()),
            DIM,
        ),
        (
            "BEST",
            player
                .timing
                .best_lap
                .map(format_time)
                .unwrap_or_else(|| "-:--.---".into()),
            PURPLE,
        ),
    ];
    for (label, value, color) in lines {
        hud.text(
            &style.regular,
            18.0 * scale,
            (tower_x + 12.0 * scale, y + 5.0 * scale),
            Align::Left,
            DIM,
            label,
        );
        hud.text(
            &style.bold,
            22.0 * scale,
            (tower_x + 288.0 * scale, y + 2.0 * scale),
            Align::Right,
            color,
            &value,
        );
        y += row;
    }

    // ---- Start lights / phase banners (top centre)
    if let Phase::Countdown { lights, .. } = race.phase {
        let spacing = 70.0 * scale;
        let left = width * 0.5 - spacing * 2.0;
        hud.fill_rect(
            left - 45.0 * scale,
            40.0 * scale,
            spacing * 4.0 + 90.0 * scale,
            90.0 * scale,
            rgba(0.0, 0.0, 0.0, 0.85),
        );
        for light in 0..5 {
            let lit = light < lights;
            hud.circle(
                (left + spacing * light as f32, 85.0 * scale),
                24.0 * scale,
                if lit {
                    rgba(1.0, 0.08, 0.06, 1.0)
                } else {
                    rgba(0.18, 0.04, 0.04, 1.0)
                },
            );
        }
    }
    if let Some(corner) = race.track.corner_near(car.track_position.s) {
        if !matches!(race.phase, Phase::Countdown { .. }) {
            hud.text(
                &style.regular,
                24.0 * scale,
                (width * 0.5, 30.0 * scale),
                Align::Center,
                rgba(1.0, 1.0, 1.0, 0.85),
                &corner.name.to_uppercase(),
            );
        }
    }
    let mut banner_y = height * 0.28;
    for (text, color, remaining) in &session.messages.active {
        let fade = remaining.min(0.4) / 0.4;
        let color = color.gamma_multiply(fade);
        let text_width = hud.text_width(&style.bold, 40.0 * scale, text);
        hud.fill_rect(
            width * 0.5 - text_width * 0.5 - 20.0 * scale,
            banner_y,
            text_width + 40.0 * scale,
            56.0 * scale,
            rgba(0.0, 0.0, 0.0, 0.55 * fade),
        );
        hud.text(
            &style.bold,
            40.0 * scale,
            (width * 0.5, banner_y + 2.0 * scale),
            Align::Center,
            color,
            text,
        );
        banner_y += 64.0 * scale;
    }
    if session.paused {
        hud.fill_rect(0.0, 0.0, width, height, rgba(0.0, 0.0, 0.0, 0.45));
        hud.text(
            &style.bold,
            72.0 * scale,
            (width * 0.5, height * 0.4),
            Align::Center,
            WHITE,
            "PAUSED",
        );
        hud.text(
            &style.regular,
            26.0 * scale,
            (width * 0.5, height * 0.4 + 90.0 * scale),
            Align::Center,
            DIM,
            "P resume   ·   R reset car   ·   C camera   ·   Esc quit",
        );
    }

    // ---- Driver panel (bottom right)
    let panel_w = 420.0 * scale;
    let panel_h = 190.0 * scale;
    let px = width - panel_w - 24.0 * scale;
    let py = height - panel_h - 24.0 * scale;
    hud.fill_rect(px, py, panel_w, panel_h, PANEL);
    let engine = &car.spec.engine;
    let rpm_fraction = ((car.powertrain.rpm - engine.idle_rpm)
        / (engine.rev_limit - engine.idle_rpm))
        .clamp(0.0, 1.0) as f32;
    // Shift lights: green, red, then blue at the optimum change point.
    let lights = 15;
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
        let flash = car.powertrain.limiter && (race.time * 12.0).sin() > 0.0;
        let color = if flash {
            BLUE
        } else if lit {
            color
        } else {
            rgba(0.15, 0.15, 0.18, 1.0)
        };
        hud.circle(
            (
                px + 26.0 * scale + light as f32 * 26.0 * scale,
                py + 22.0 * scale,
            ),
            9.0 * scale,
            color,
        );
    }
    let gear = match car.powertrain.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        gear => gear.to_string(),
    };
    hud.text(
        &style.bold,
        110.0 * scale,
        (px + 90.0 * scale, py + 36.0 * scale),
        Align::Center,
        WHITE,
        &gear,
    );
    let kmh = (car.speed() * 3.6).round() as i32;
    hud.text(
        &style.bold,
        72.0 * scale,
        (px + 330.0 * scale, py + 44.0 * scale),
        Align::Right,
        WHITE,
        &kmh.to_string(),
    );
    hud.text(
        &style.regular,
        22.0 * scale,
        (px + 340.0 * scale, py + 86.0 * scale),
        Align::Left,
        DIM,
        "KM/H",
    );
    hud.text(
        &style.regular,
        20.0 * scale,
        (px + 330.0 * scale, py + 126.0 * scale),
        Align::Right,
        DIM,
        &format!("{:>5.0} RPM", car.powertrain.rpm),
    );
    // Pedals.
    let bar_y = py + panel_h - 26.0 * scale;
    let bar_w = (panel_w - 48.0 * scale) * 0.5;
    hud.fill_rect(
        px + 16.0 * scale,
        bar_y,
        bar_w,
        10.0 * scale,
        rgba(0.15, 0.15, 0.18, 1.0),
    );
    hud.fill_rect(
        px + 16.0 * scale,
        bar_y,
        bar_w * session.controls.brake() as f32,
        10.0 * scale,
        ACCENT,
    );
    hud.fill_rect(
        px + 32.0 * scale + bar_w,
        bar_y,
        bar_w,
        10.0 * scale,
        rgba(0.15, 0.15, 0.18, 1.0),
    );
    hud.fill_rect(
        px + 32.0 * scale + bar_w,
        bar_y,
        bar_w * session.controls.throttle() as f32,
        10.0 * scale,
        GREEN,
    );
    let steer = session.controls.steer() as f32;
    let centre = px + panel_w * 0.5;
    hud.fill_rect(
        centre - 1.0 * scale,
        bar_y - 18.0 * scale,
        2.0 * scale,
        12.0 * scale,
        DIM,
    );
    hud.fill_rect(
        centre - steer * panel_w * 0.4 - 4.0 * scale,
        bar_y - 18.0 * scale,
        8.0 * scale,
        12.0 * scale,
        WHITE,
    );
    if let Some(refusal) = car.powertrain.last_refusal {
        let text = match refusal {
            ShiftRefusal::OverRev => "OVER-REV",
            ShiftRefusal::TooFast => "TOO FAST",
        };
        hud.text(
            &style.bold,
            26.0 * scale,
            (px + 90.0 * scale, py + 150.0 * scale),
            Align::Center,
            AMBER,
            text,
        );
    }

    // ---- Car state (bottom left): tyres, aero, assists
    let cx = 24.0 * scale;
    let cy = height - 214.0 * scale;
    hud.fill_rect(cx, cy, 340.0 * scale, 190.0 * scale, PANEL);
    let compound = car.setup.compound;
    let properties = compound.properties();
    hud.text(
        &style.bold,
        22.0 * scale,
        (cx + 14.0 * scale, cy + 8.0 * scale),
        Align::Left,
        WHITE,
        &format!("TYRES · {}", compound.label()),
    );
    for (axle, label) in ["FRONT", "REAR"].iter().enumerate() {
        let tyre = car.tyres[axle];
        let offset = tyre.temperature - properties.optimal_temperature;
        let color = if offset < -properties.window {
            BLUE
        } else if offset > properties.window {
            ACCENT
        } else {
            GREEN
        };
        let row_y = cy + 44.0 * scale + axle as f32 * 34.0 * scale;
        hud.text(
            &style.regular,
            20.0 * scale,
            (cx + 14.0 * scale, row_y),
            Align::Left,
            DIM,
            label,
        );
        let wheels = if axle == 0 { [0, 1] } else { [2, 3] };
        for (side, &wheel) in wheels.iter().enumerate() {
            let slide = car.wheels[wheel].sliding as f32;
            let box_x = cx + 92.0 * scale + side as f32 * 36.0 * scale;
            hud.fill_rect(
                box_x,
                row_y + 2.0 * scale,
                28.0 * scale,
                26.0 * scale,
                color,
            );
            if car.wheels[wheel].locked || slide > 0.3 {
                hud.stroke_rect(
                    box_x,
                    row_y + 2.0 * scale,
                    28.0 * scale,
                    26.0 * scale,
                    3.0 * scale,
                    WHITE,
                );
            }
        }
        hud.text(
            &style.bold,
            22.0 * scale,
            (cx + 230.0 * scale, row_y),
            Align::Right,
            color,
            &format!("{:.0}°C", tyre.temperature),
        );
        hud.text(
            &style.regular,
            20.0 * scale,
            (cx + 326.0 * scale, row_y + 2.0 * scale),
            Align::Right,
            DIM,
            &format!("{:.0}% worn", tyre.wear * 100.0),
        );
    }
    let meter = |hud: &mut Hud, label: &str, value: f64, max: f64, y: f32, color: Color| {
        hud.text(
            &style.regular,
            18.0 * scale,
            (cx + 14.0 * scale, y),
            Align::Left,
            DIM,
            label,
        );
        let bar_x = cx + 130.0 * scale;
        let bar_w = 196.0 * scale;
        hud.fill_rect(
            bar_x,
            y + 8.0 * scale,
            bar_w,
            8.0 * scale,
            rgba(0.15, 0.15, 0.18, 1.0),
        );
        hud.fill_rect(
            bar_x,
            y + 8.0 * scale,
            bar_w * (value / max).clamp(0.0, 1.0) as f32,
            8.0 * scale,
            color,
        );
    };
    meter(
        hud,
        "SLIPSTREAM",
        car.aero.slipstream(),
        0.24,
        cy + 118.0 * scale,
        BLUE,
    );
    meter(
        hud,
        "DIRTY AIR",
        car.aero.dirty_air(),
        0.24,
        cy + 142.0 * scale,
        AMBER,
    );
    hud.text(
        &style.regular,
        18.0 * scale,
        (cx + 14.0 * scale, cy + 162.0 * scale),
        Align::Left,
        DIM,
        &format!(
            "WINGS F{:.0} R{:.0}   {}   {}",
            car.setup.front_wing,
            car.setup.rear_wing,
            if car.assists.automatic_gears {
                "AUTO"
            } else {
                "MANUAL"
            },
            match session.camera.mode {
                Mode::Chase => "CHASE",
                Mode::Far => "FAR",
                Mode::TCam => "T-CAM",
                Mode::Nose => "NOSE",
            }
        ),
    );

    // ---- Minimap (top right)
    let size = 240.0 * scale;
    let mx = width - size - 24.0 * scale;
    let my = 24.0 * scale;
    hud.fill_rect(mx, my, size, size, PANEL);
    let inset = 14.0 * scale;
    let map = |point: (f32, f32)| {
        (
            mx + inset + point.0 * (size - inset * 2.0),
            my + inset + point.1 * (size - inset * 2.0),
        )
    };
    let points = &session.minimap.points;
    for index in 0..points.len() {
        let a = map(points[index]);
        let b = map(points[(index + 1) % points.len()]);
        hud.line(a, b, 3.0 * scale, rgba(0.8, 0.82, 0.86, 0.9));
    }
    let start = map(points[0]);
    hud.fill_rect(
        start.0 - 2.0 * scale,
        start.1 - 6.0 * scale,
        4.0 * scale,
        12.0 * scale,
        WHITE,
    );
    for (index, entrant) in race.entrants.iter().enumerate() {
        if index == session.player {
            continue;
        }
        let point = map(session
            .minimap
            .project(entrant.car.position.x, entrant.car.position.z));
        hud.circle(point, 5.0 * scale, DIM);
    }
    let me = map(session.minimap.project(car.position.x, car.position.z));
    hud.circle(me, 7.0 * scale, ACCENT);

    // ---- Results
    if race.phase == Phase::Finished {
        let panel_w = 560.0 * scale;
        let left = width * 0.5 - panel_w * 0.5;
        let top = height * 0.18;
        let rows = race.entrants.len() as f32;
        hud.fill_rect(
            left,
            top,
            panel_w,
            (90.0 + rows * 34.0) * scale,
            rgba(0.02, 0.025, 0.035, 0.9),
        );
        hud.text(
            &style.bold,
            40.0 * scale,
            (width * 0.5, top + 14.0 * scale),
            Align::Center,
            WHITE,
            "RESULTS",
        );
        for (rank, &index) in race.order().iter().enumerate() {
            let entrant = &race.entrants[index];
            let row_y = top + (72.0 + rank as f32 * 34.0) * scale;
            let color = if index == session.player {
                AMBER
            } else {
                WHITE
            };
            hud.text(
                &style.bold,
                24.0 * scale,
                (left + 24.0 * scale, row_y),
                Align::Left,
                color,
                &format!("{}. {}", rank + 1, entrant.name),
            );
            let best = entrant.timing.best_lap.map(format_time).unwrap_or_default();
            hud.text(
                &style.regular,
                22.0 * scale,
                (left + panel_w - 24.0 * scale, row_y + 2.0 * scale),
                Align::Right,
                DIM,
                &format!("best {best}"),
            );
        }
    }
}
