//! Race HUD drawn through egui in Bevy.

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

const WHITE: Color = Color::WHITE;
const DIM: Color = Color::from_rgb(160, 166, 178);
const ACCENT: Color = Color::from_rgb(237, 33, 43);
const PURPLE: Color = Color::from_rgb(184, 92, 255);
const GREEN: Color = Color::from_rgb(64, 230, 115);
const AMBER: Color = Color::from_rgb(255, 184, 38);
const BLUE: Color = Color::from_rgb(77, 153, 255);
const TYRE_YELLOW: Color = Color::from_rgb(255, 220, 0);

// Instrument surfaces: a charcoal housing holding near-black inset readouts.
const HOUSING: Color = Color::from_rgba_premultiplied(37, 38, 42, 232);
const HOUSING_EDGE: Color = Color::from_rgb(74, 76, 83);
const WELL: Color = Color::from_rgb(9, 10, 12);
const WELL_RIM: Color = Color::from_rgb(56, 58, 64);
const UNLIT: Color = Color::from_rgb(46, 47, 50);
const LABEL: Color = Color::from_rgb(120, 125, 136);

/// Thermal state of a tyre relative to its compound's working window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TyreBand {
    Cold,
    Optimal,
    LiftAndCoast,
    Cooked,
}

impl TyreBand {
    fn of(temperature: f64, compound: racecraft_sim::car::spec::Compound) -> Self {
        let properties = compound.properties();
        let offset = temperature - properties.optimal_temperature;
        if offset < -properties.window {
            Self::Cold
        } else if offset <= properties.window {
            Self::Optimal
        } else if offset <= properties.window * 2.0 {
            Self::LiftAndCoast
        } else {
            Self::Cooked
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Cold => BLUE,
            Self::Optimal => GREEN,
            Self::LiftAndCoast => TYRE_YELLOW,
            Self::Cooked => ACCENT,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Cold => "COLD",
            Self::Optimal => "OPTIMAL",
            Self::LiftAndCoast => "LIFT & COAST",
            Self::Cooked => "COOKED",
        }
    }
}

/// HUD units per pixel. Designed at 1080p; narrow or tiled windows shrink by
/// width as well, so the corner instruments leave the cockpit wheel in view.
pub fn scale(size: egui::Vec2) -> f32 {
    (size.y / 1080.0).min(size.x / 1500.0)
}

/// Raised charcoal housing that groups related instruments.
fn housing(hud: &Hud, x: f32, y: f32, w: f32, h: f32, scale: f32) {
    hud.fill_round(x, y, w, h, 12.0 * scale, HOUSING);
    hud.stroke_round(x, y, w, h, 12.0 * scale, 1.5 * scale, HOUSING_EDGE);
}

/// Recessed black readout window.
fn well(hud: &Hud, x: f32, y: f32, w: f32, h: f32, scale: f32) {
    hud.fill_round(x, y, w, h, 6.0 * scale, WELL);
    hud.stroke_round(x, y, w, h, 6.0 * scale, 1.0 * scale, WELL_RIM);
}

/// Small caption printed on a housing or inside a readout.
fn caption(hud: &Hud, style: &HudStyle, size: f32, at: (f32, f32), align: Align, text: &str) {
    hud.text(&style.bold, size, at, align, LABEL, text);
}

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
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
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
    if session.ready
        && matches!(
            session.screen,
            crate::menu::Screen::Racing | crate::menu::Screen::CameraEditor
        )
    {
        draw(&mut hud, &style, &mut session, time.delta_secs());
    } else if !session.ready {
        hud.text(
            &style.bold,
            32.0,
            (hud.width() * 0.5, hud.height() * 0.45),
            Align::Center,
            WHITE,
            "LOADING TRACK AND CARS",
        );
    }
    if session.show_performance {
        use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
        let frame = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME);
        let average = frame.and_then(|d| d.smoothed()).unwrap_or(0.0);
        let worst = frame
            .map(|d| d.values().copied().fold(0.0, f64::max))
            .unwrap_or(0.0);
        let scale = scale(hud.size);
        hud.text(
            &style.regular,
            18.0 * scale,
            (hud.width() * 0.5, hud.height() - 30.0 * scale),
            Align::Center,
            AMBER,
            &format!(
                "{:.0} FPS   {average:.1} ms avg   {worst:.1} ms worst",
                1000.0 / average.max(0.01)
            ),
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

/// Live delta against the player's best lap, sampled per centerline station.
#[derive(Default)]
pub struct Delta {
    best: Vec<f64>,
    lap: Vec<f64>,
    laps: u32,
    current: Option<f64>,
}

impl Delta {
    fn update(&mut self, race: &racecraft_sim::Race, player: usize) {
        let timing = &race.entrants[player].timing;
        let count = race.track.stations.len();
        if self.lap.len() != count {
            self.lap = vec![f64::NAN; count];
        }
        if timing.laps_completed != self.laps {
            let filled = self.lap.iter().filter(|t| t.is_finite()).count();
            if timing.laps_completed > self.laps
                && timing.last_lap.is_some()
                && timing.last_lap == timing.best_lap
                && filled * 10 > count * 9
            {
                // Carry samples over stations skipped by a fast frame.
                let mut last = 0.0;
                for time in &mut self.lap {
                    if time.is_finite() {
                        last = *time;
                    } else {
                        *time = last;
                    }
                }
                self.best = std::mem::replace(&mut self.lap, vec![f64::NAN; count]);
            } else {
                self.lap.fill(f64::NAN);
            }
            self.laps = timing.laps_completed;
        }
        self.current = None;
        if !matches!(race.phase, Phase::Racing | Phase::Finishing)
            || timing.finish_time.is_some()
            || timing.distance < 0.0
        {
            return;
        }
        let elapsed = race.time - timing.lap_start;
        let station = race.track.station_at(timing.distance);
        if self.lap[station].is_nan() {
            self.lap[station] = elapsed;
        }
        if let Some(&best) = self.best.get(station) {
            self.current = Some(elapsed - best);
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
    session.delta.update(&session.race, session.player);
    if session.race.phase == Phase::Finished {
        session.messages.active.clear();
    }
    for message in &mut session.messages.active {
        message.2 -= dt;
    }
    session.messages.active.retain(|message| message.2 > 0.0);

    let scale = scale(hud.size);
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

    // ---- Timing tower (top left). Its row heights are mirrored by the
    // race-controls strip placed underneath it in menu.rs.
    let tower_x = 24.0 * scale;
    let tower_w = 300.0 * scale;
    let mut y = 24.0 * scale;
    let row = 30.0 * scale;
    let tower_h = row * 1.5 + race.entrants.len() as f32 * row;
    housing(hud, tower_x, y, tower_w, tower_h, scale);
    hud.fill_round(
        tower_x + 10.0 * scale,
        y + 12.0 * scale,
        4.0 * scale,
        row * 1.5 - 22.0 * scale,
        2.0 * scale,
        ACCENT,
    );
    caption(
        hud,
        style,
        18.0 * scale,
        (tower_x + 22.0 * scale, y + 10.0 * scale),
        Align::Left,
        "STANDINGS",
    );
    hud.text(
        &style.bold,
        22.0 * scale,
        (tower_x + tower_w - 12.0 * scale, y + 7.0 * scale),
        Align::Right,
        WHITE,
        &format!("LAP {lap}/{}", race.laps),
    );
    y += row * 1.5;
    let leader_distance = race.entrants[race.order()[0]].timing.distance;
    for (rank, &index) in race.order().iter().enumerate() {
        let entrant = &race.entrants[index];
        let mine = index == session.player;
        if mine {
            hud.fill_round(
                tower_x + 6.0 * scale,
                y + 2.0 * scale,
                tower_w - 12.0 * scale,
                row - 4.0 * scale,
                4.0 * scale,
                WELL,
            );
            hud.stroke_round(
                tower_x + 6.0 * scale,
                y + 2.0 * scale,
                tower_w - 12.0 * scale,
                row - 4.0 * scale,
                4.0 * scale,
                1.0 * scale,
                ACCENT,
            );
        }
        hud.text(
            &style.bold,
            20.0 * scale,
            (tower_x + 34.0 * scale, y + 3.0 * scale),
            Align::Right,
            if mine { WHITE } else { DIM },
            &(rank + 1).to_string(),
        );
        let team = if entrant.car.spec.name == "RB22" {
            rgba(0.2, 0.3, 0.9, 1.0)
        } else {
            rgba(0.9, 0.75, 0.1, 1.0)
        };
        hud.fill_round(
            tower_x + 42.0 * scale,
            y + 7.0 * scale,
            4.0 * scale,
            row - 14.0 * scale,
            2.0 * scale,
            team,
        );
        hud.text(
            &style.regular,
            20.0 * scale,
            (tower_x + 54.0 * scale, y + 3.0 * scale),
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
                // Each chaser covers the gap at its own pace, not the player's.
                format!("+{:.1}", metres / entrant.car.speed().max(30.0))
            }
        };
        hud.text(
            &style.regular,
            18.0 * scale,
            (tower_x + tower_w - 14.0 * scale, y + 4.0 * scale),
            Align::Right,
            DIM,
            &gap,
        );
        y += row;
    }
    // Lap times under the tower.
    y += 10.0 * scale;
    housing(hud, tower_x, y, tower_w, row * 3.0, scale);
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
        caption(
            hud,
            style,
            16.0 * scale,
            (tower_x + 16.0 * scale, y + 6.0 * scale),
            Align::Left,
            label,
        );
        hud.text(
            &style.bold,
            22.0 * scale,
            (tower_x + tower_w - 14.0 * scale, y + 2.0 * scale),
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
        housing(
            hud,
            left - 45.0 * scale,
            40.0 * scale,
            spacing * 4.0 + 90.0 * scale,
            90.0 * scale,
            scale,
        );
        for light in 0..5 {
            let lit = light < lights;
            let centre = (left + spacing * light as f32, 85.0 * scale);
            hud.circle(centre, 27.0 * scale, WELL);
            hud.circle(
                centre,
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
    if let Some(delta) = session.delta.current {
        let text = format!("{delta:+.3}");
        let box_w = 150.0 * scale;
        let box_x = width * 0.5 - box_w * 0.5;
        let color = if delta <= 0.0 { GREEN } else { ACCENT };
        hud.fill_round(box_x, 62.0 * scale, box_w, 40.0 * scale, 6.0 * scale, WELL);
        hud.stroke_round(
            box_x,
            62.0 * scale,
            box_w,
            40.0 * scale,
            6.0 * scale,
            2.0 * scale,
            color,
        );
        hud.text(
            &style.bold,
            30.0 * scale,
            (width * 0.5, 63.0 * scale),
            Align::Center,
            color,
            &text,
        );
    }
    let mut banner_y = height * 0.28;
    for (text, color, remaining) in &session.messages.active {
        let fade = remaining.min(0.4) / 0.4;
        let color = color.gamma_multiply(fade);
        let text_width = hud.text_width(&style.bold, 40.0 * scale, text);
        let banner_x = width * 0.5 - text_width * 0.5 - 24.0 * scale;
        let banner_w = text_width + 48.0 * scale;
        hud.fill_round(
            banner_x,
            banner_y,
            banner_w,
            56.0 * scale,
            8.0 * scale,
            HOUSING.gamma_multiply(fade),
        );
        hud.fill_round(
            banner_x,
            banner_y + 10.0 * scale,
            4.0 * scale,
            36.0 * scale,
            2.0 * scale,
            color,
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

    // ---- Dash (bottom right): RPM LEDs, lap / gear / speed, lap time and
    // position, then pedal traces.
    let pad = 14.0 * scale;
    let gap = 10.0 * scale;
    let dash_w = 508.0 * scale;
    let led_h = 34.0 * scale;
    let main_h = 78.0 * scale;
    let lower_h = 46.0 * scale;
    let dash_h = pad + led_h + gap + main_h + gap + lower_h + 40.0 * scale;
    let px = width - dash_w - 24.0 * scale;
    let py = height - dash_h - 24.0 * scale;
    housing(hud, px, py, dash_w, dash_h, scale);
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
            UNLIT
        };
        let centre = (
            inner_x + led_h * 0.5 + light as f32 * led_pitch,
            led_y + led_h * 0.5,
        );
        hud.circle(centre, 11.0 * scale, color);
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
    let value_y = main_y + 14.0 * scale;
    let value_size = 52.0 * scale;
    hud.text(
        &style.bold,
        value_size,
        (inner_x + side_w - 12.0 * scale, value_y),
        Align::Right,
        WHITE,
        &format!("{lap}/{}", race.laps),
    );
    // Gear, framed by a red bezel (amber while a shift is being refused).
    let refusal = car.powertrain.last_refusal.map(|refusal| match refusal {
        ShiftRefusal::OverRev => "OVER-REV",
        ShiftRefusal::TooFast => "TOO FAST",
    });
    let bezel = if refusal.is_some() { AMBER } else { ACCENT };
    hud.fill_round(gear_x, main_y, gear_w, main_h, 8.0 * scale, WELL);
    hud.stroke_round(
        gear_x,
        main_y,
        gear_w,
        main_h,
        8.0 * scale,
        4.0 * scale,
        bezel,
    );
    let gear = match car.powertrain.gear {
        -1 => "R".to_string(),
        0 => "N".to_string(),
        gear => gear.to_string(),
    };
    let gear_size = 76.0 * scale;
    let gear_y = main_y - 10.0 * scale;
    hud.text(
        &style.bold,
        gear_size,
        (gear_x + gear_w * 0.5, gear_y),
        Align::Center,
        WHITE,
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
        &format!("{:.0} RPM", car.powertrain.rpm),
    );
    let kmh = (car.speed() * 3.6).round() as i32;
    hud.text(
        &style.bold,
        value_size,
        (speed_x + side_w - 12.0 * scale, value_y),
        Align::Right,
        WHITE,
        &kmh.to_string(),
    );

    // Lower row: current lap time where a fuel readout would sit, position
    // under the speed window.
    let lower_y = main_y + main_h + gap;
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
    let lower_size = 32.0 * scale;
    let lower_value_y = lower_y + 1.0 * scale;
    hud.text(
        &style.bold,
        lower_size,
        (inner_x + time_w - 12.0 * scale, lower_value_y),
        Align::Right,
        WHITE,
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
    hud.text(
        &style.bold,
        lower_size,
        (speed_x + side_w - 12.0 * scale, lower_value_y),
        Align::Right,
        WHITE,
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

    // Pedals and steering.
    let steer_y = lower_y + lower_h + 10.0 * scale;
    let bar_y = steer_y + 14.0 * scale;
    let bar_w = (inner_w - gap) * 0.5;
    let centre = inner_x + inner_w * 0.5;
    hud.fill_round(
        inner_x,
        steer_y + 4.0 * scale,
        inner_w,
        2.0 * scale,
        1.0 * scale,
        WELL_RIM,
    );
    hud.fill_rect(
        centre - 1.0 * scale,
        steer_y,
        2.0 * scale,
        10.0 * scale,
        LABEL,
    );
    let steer = session.guidance.activity.input.steer as f32;
    hud.fill_round(
        centre - steer * inner_w * 0.5 - 4.0 * scale,
        steer_y,
        8.0 * scale,
        10.0 * scale,
        2.0 * scale,
        WHITE,
    );
    let brake = session.guidance.activity.input.brake as f32;
    let throttle = session.guidance.activity.input.throttle as f32;
    hud.fill_round(inner_x, bar_y, bar_w, 8.0 * scale, 4.0 * scale, WELL);
    hud.fill_round(
        inner_x + bar_w * (1.0 - brake),
        bar_y,
        bar_w * brake,
        8.0 * scale,
        4.0 * scale,
        ACCENT,
    );
    hud.fill_round(
        inner_x + bar_w + gap,
        bar_y,
        bar_w,
        8.0 * scale,
        4.0 * scale,
        WELL,
    );
    hud.fill_round(
        inner_x + bar_w + gap,
        bar_y,
        bar_w * throttle,
        8.0 * scale,
        4.0 * scale,
        GREEN,
    );

    // ---- Car state (bottom left): aids strip, then tyres and aero.
    let cx = 24.0 * scale;
    let car_w = 392.0 * scale;
    let car_h = 214.0 * scale;
    let cy = height - car_h - 24.0 * scale;
    housing(hud, cx, cy - 76.0 * scale, car_w, 66.0 * scale, scale);
    if session.corner_braking > 0. {
        caption(
            hud,
            style,
            15.0 * scale,
            (cx + 14.0 * scale, cy - 68.0 * scale),
            Align::Left,
            &format!(
                "CORNER TARGET  {:.0} KM/H",
                session.guidance.activity.target_speed * 3.6
            ),
        );
    }
    for (index, label, configured, active) in [
        (
            0,
            "STEER",
            car.assists.steering > 0.,
            session.guidance.activity.steering,
        ),
        (
            1,
            "BRAKE",
            session.corner_braking > 0.,
            session.guidance.activity.braking,
        ),
        (
            2,
            "AUTO",
            car.assists.automatic_gears,
            car.assists.automatic_gears,
        ),
    ] {
        let tag_w = 112.0 * scale;
        let tag_x = cx + 14.0 * scale + index as f32 * (tag_w + 9.0 * scale);
        let tag_y = cy - 44.0 * scale;
        let color = if active {
            GREEN
        } else if configured {
            DIM
        } else {
            Color::from_rgb(70, 74, 82)
        };
        hud.fill_round(tag_x, tag_y, tag_w, 26.0 * scale, 5.0 * scale, WELL);
        hud.stroke_round(
            tag_x,
            tag_y,
            tag_w,
            26.0 * scale,
            5.0 * scale,
            1.0 * scale,
            if active { GREEN } else { WELL_RIM },
        );
        hud.text(
            &style.bold,
            16.0 * scale,
            (tag_x + tag_w * 0.5, tag_y + 1.0 * scale),
            Align::Center,
            color,
            label,
        );
    }
    housing(hud, cx, cy, car_w, car_h, scale);
    let compound = car.setup.compound;
    caption(
        hud,
        style,
        18.0 * scale,
        (cx + 14.0 * scale, cy + 10.0 * scale),
        Align::Left,
        "TYRES",
    );
    hud.text(
        &style.bold,
        20.0 * scale,
        (cx + 76.0 * scale, cy + 8.0 * scale),
        Align::Left,
        WHITE,
        compound.label(),
    );
    caption(
        hud,
        style,
        14.0 * scale,
        (cx + car_w - 14.0 * scale, cy + 13.0 * scale),
        Align::Right,
        "WEAR",
    );
    let tyre_w = car_w - 28.0 * scale;
    well(
        hud,
        cx + 14.0 * scale,
        cy + 40.0 * scale,
        tyre_w,
        74.0 * scale,
        scale,
    );
    for (axle, label) in ["FRONT", "REAR"].iter().enumerate() {
        let tyre = car.tyres[axle];
        let band = TyreBand::of(tyre.temperature, compound);
        let color = band.color();
        let row_y = cy + 45.0 * scale + axle as f32 * 34.0 * scale;
        caption(
            hud,
            style,
            15.0 * scale,
            (cx + 26.0 * scale, row_y + 6.0 * scale),
            Align::Left,
            label,
        );
        let wheels = if axle == 0 { [0, 1] } else { [2, 3] };
        for (side, &wheel) in wheels.iter().enumerate() {
            let slide = car.wheels[wheel].sliding as f32;
            let box_x = cx + 86.0 * scale + side as f32 * 30.0 * scale;
            hud.fill_round(
                box_x,
                row_y + 3.0 * scale,
                22.0 * scale,
                26.0 * scale,
                5.0 * scale,
                color,
            );
            if car.wheels[wheel].locked || slide > 0.3 {
                hud.stroke_round(
                    box_x,
                    row_y + 3.0 * scale,
                    22.0 * scale,
                    26.0 * scale,
                    5.0 * scale,
                    3.0 * scale,
                    WHITE,
                );
            }
        }
        hud.text(
            &style.bold,
            22.0 * scale,
            (cx + 202.0 * scale, row_y),
            Align::Right,
            color,
            &format!("{:.0}°C", tyre.temperature),
        );
        hud.text(
            &style.bold,
            15.0 * scale,
            (cx + 212.0 * scale, row_y + 6.0 * scale),
            Align::Left,
            color,
            band.label(),
        );
        hud.text(
            &style.regular,
            20.0 * scale,
            (cx + car_w - 26.0 * scale, row_y + 1.0 * scale),
            Align::Right,
            DIM,
            &format!("{:.0}%", tyre.wear * 100.0),
        );
    }
    let meter = |hud: &mut Hud, label: &str, value: f64, max: f64, y: f32, color: Color| {
        caption(
            hud,
            style,
            15.0 * scale,
            (cx + 14.0 * scale, y),
            Align::Left,
            label,
        );
        let bar_x = cx + 130.0 * scale;
        let bar_w = car_w - 144.0 * scale;
        hud.fill_round(
            bar_x,
            y + 6.0 * scale,
            bar_w,
            8.0 * scale,
            4.0 * scale,
            WELL,
        );
        hud.fill_round(
            bar_x,
            y + 6.0 * scale,
            bar_w * (value / max).clamp(0.0, 1.0) as f32,
            8.0 * scale,
            4.0 * scale,
            color,
        );
    };
    meter(
        hud,
        "SLIPSTREAM",
        car.aero.slipstream(),
        0.24,
        cy + 124.0 * scale,
        BLUE,
    );
    meter(
        hud,
        "DIRTY AIR",
        car.aero.dirty_air(),
        0.24,
        cy + 148.0 * scale,
        AMBER,
    );
    caption(
        hud,
        style,
        15.0 * scale,
        (cx + 14.0 * scale, cy + 180.0 * scale),
        Align::Left,
        &format!(
            "WINGS F{:.0} R{:.0}   {}   {}",
            car.setup.front_wing,
            car.setup.rear_wing,
            if car.assists.automatic_gears {
                "AUTO"
            } else {
                "MANUAL"
            },
            session.camera.mode.label().to_uppercase()
        ),
    );

    // ---- Minimap (top right)
    let size = 240.0 * scale;
    let mx = width - size - 24.0 * scale;
    let my = 24.0 * scale;
    housing(hud, mx, my, size, size, scale);
    well(
        hud,
        mx + 8.0 * scale,
        my + 8.0 * scale,
        size - 16.0 * scale,
        size - 16.0 * scale,
        scale,
    );
    let inset = 20.0 * scale;
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
    hud.circle(me, 8.0 * scale, WELL);
    hud.circle(me, 6.0 * scale, ACCENT);

    // ---- Results
    if race.phase == Phase::Finished {
        let panel_w = 560.0 * scale;
        // Clear the timing tower, its control strip and the minimap.
        let left = (width * 0.5 - panel_w * 0.5).max(tower_x + tower_w + 16.0 * scale);
        let top = (height * 0.18).max(my + size + 16.0 * scale);
        let rows = race.entrants.len() as f32;
        housing(hud, left, top, panel_w, (96.0 + rows * 34.0) * scale, scale);
        hud.fill_round(
            left + 24.0 * scale,
            top + 22.0 * scale,
            4.0 * scale,
            32.0 * scale,
            2.0 * scale,
            ACCENT,
        );
        hud.text(
            &style.bold,
            36.0 * scale,
            (left + 38.0 * scale, top + 12.0 * scale),
            Align::Left,
            WHITE,
            "RESULTS",
        );
        caption(
            hud,
            style,
            15.0 * scale,
            (left + panel_w - 24.0 * scale, top + 30.0 * scale),
            Align::Right,
            "BEST LAP",
        );
        well(
            hud,
            left + 14.0 * scale,
            top + 70.0 * scale,
            panel_w - 28.0 * scale,
            (rows * 34.0 + 12.0) * scale,
            scale,
        );
        for (rank, &index) in race.order().iter().enumerate() {
            let entrant = &race.entrants[index];
            let row_y = top + (76.0 + rank as f32 * 34.0) * scale;
            let mine = index == session.player;
            if mine {
                hud.fill_round(
                    left + 20.0 * scale,
                    row_y,
                    4.0 * scale,
                    30.0 * scale,
                    2.0 * scale,
                    ACCENT,
                );
            }
            let color = if mine { WHITE } else { DIM };
            hud.text(
                &style.bold,
                24.0 * scale,
                (left + 62.0 * scale, row_y),
                Align::Right,
                color,
                &format!("P{}", rank + 1),
            );
            hud.text(
                &style.bold,
                24.0 * scale,
                (left + 78.0 * scale, row_y),
                Align::Left,
                color,
                &entrant.name.to_uppercase(),
            );
            let best = entrant
                .timing
                .best_lap
                .map(format_time)
                .unwrap_or_else(|| "-:--.---".into());
            hud.text(
                &style.regular,
                22.0 * scale,
                (left + panel_w - 28.0 * scale, row_y + 2.0 * scale),
                Align::Right,
                color,
                &best,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use racecraft_sim::car::spec::Compound;

    #[test]
    fn tyre_colors_follow_each_compounds_thermal_window() {
        let labels: Vec<_> = [
            TyreBand::Cold,
            TyreBand::Optimal,
            TyreBand::LiftAndCoast,
            TyreBand::Cooked,
        ]
        .map(TyreBand::label)
        .into();
        assert_eq!(labels, ["COLD", "OPTIMAL", "LIFT & COAST", "COOKED"]);
        for compound in Compound::ALL {
            let properties = compound.properties();
            let optimal = properties.optimal_temperature;
            let window = properties.window;
            for (temperature, expected) in [
                (optimal - window - 0.1, BLUE),
                (optimal - window, GREEN),
                (optimal, GREEN),
                (optimal + window, GREEN),
                (optimal + window + 0.1, TYRE_YELLOW),
                (optimal + window * 2.0, TYRE_YELLOW),
                (optimal + window * 2.0 + 0.1, ACCENT),
            ] {
                assert_eq!(TyreBand::of(temperature, compound).color(), expected);
            }
        }
    }
}
