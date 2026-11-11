//! Race HUD drawn through egui in Bevy.

mod instruments;
#[cfg(test)]
mod proof;
mod telemetry;
mod timing;
pub use telemetry::Telemetry;
pub mod wheel;

use crate::canvas::{rgba, Align, Finish, Hud};
use crate::Session;
use bevy::prelude::*;
use bevy_egui::{
    egui::{self, Color32 as Color, FontFamily},
    EguiContexts,
};
use racecraft_sim::race::{Phase, RaceEvent};
use racecraft_sim::Track;
use serde::{Deserialize, Serialize};

impl Session {
    /// Called once after each physics step, before either display renders.
    pub fn sample_hud(&mut self) {
        self.telemetry.update(&self.race, self.player);
        self.telemetry.pedals(self.guidance.activity.input);
        self.delta.update(&self.race, self.player);
    }

    pub fn recover_hud(&mut self) {
        self.telemetry.recover();
        self.delta.lap.fill(f64::NAN);
        self.delta.previous = None;
        self.delta.average = Default::default();
        self.delta.current = None;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Segments,
    Gauges,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapMode {
    #[default]
    FullCircuit,
    Nearby,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub layout: Layout,
    pub map: MapMode,
}

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

const WELL: Color = Color::from_rgb(9, 10, 12);
const WELL_RIM: Color = Color::from_rgb(56, 58, 64);

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
}

/// HUD units per pixel. Designed at 1080p; narrow or tiled windows shrink by
/// width as well, so the corner instruments leave the cockpit wheel in view.
pub fn scale(size: egui::Vec2) -> f32 {
    (size.y / 1080.0).min(size.x / 1500.0)
}

/// Raised charcoal housing that groups related instruments.
fn housing(hud: &Hud, x: f32, y: f32, w: f32, h: f32, scale: f32) {
    let colors = hud.finish.colors();
    let cut = match hud.finish {
        Finish::Digital => 6.,
        Finish::Analog => 12.,
    };
    hud.plate(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
        cut * scale,
        colors.top,
        colors.bottom,
        colors.edge,
    );
}

/// Recessed black readout window.
fn well(hud: &Hud, x: f32, y: f32, w: f32, h: f32, scale: f32) {
    let colors = hud.finish.colors();
    hud.plate(
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
        3. * scale,
        Color::from_rgb(3, 7, 8),
        Color::from_rgb(10, 17, 19),
        colors.edge.gamma_multiply(0.7),
    );
}

/// Small caption printed on a housing or inside a readout.
fn caption(hud: &Hud, style: &HudStyle, size: f32, at: (f32, f32), align: Align, text: &str) {
    hud.text(
        &style.bold,
        size,
        at,
        align,
        hud.finish.colors().muted,
        text,
    );
}

/// Number centred vertically on `at.1`, so it shares a midline with labels.
fn readout(
    hud: &Hud,
    style: &HudStyle,
    height: f32,
    at: (f32, f32),
    align: Align,
    color: Color,
    text: &str,
) {
    // The LCD theme carries every number on screen; words and the analog
    // binnacle stay typographic.
    if matches!(hud.finish, Finish::Digital) && Hud::can_segment(text) {
        hud.segments(height, (at.0, at.1 - height * 0.5), align, color, text);
        return;
    }
    hud.text_mid(&style.regular, height * 1.3, at, align, color, text);
}

/// Titillium Web for every egui context: the screen HUD and the wheel LCD.
pub fn fonts() -> egui::FontDefinitions {
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
    fonts
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
    presentation: (
        Res<crate::menu::Frontend>,
        Option<Res<crate::race_loop::Run>>,
    ),
) -> Result {
    let (frontend, rewards) = presentation;
    let context = contexts.ctx_mut()?;
    if !*initialized {
        context.set_fonts(fonts());
        *initialized = true;
        return Ok(());
    }
    let mut hud = Hud {
        painter: context.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("race-hud"),
        )),
        size: context.content_rect().size(),
        finish: match frontend.display.layout {
            Layout::Segments => Finish::Digital,
            Layout::Gauges => Finish::Analog,
        },
    };
    if session.ready
        && !(session.pings.is_empty() && rewards.as_ref().is_some_and(|r| r.summary()))
        && matches!(
            session.screen,
            crate::menu::Screen::Racing | crate::menu::Screen::CameraEditor
        )
    {
        draw(
            &mut hud,
            &style,
            &mut session,
            frontend.display,
            time.delta_secs(),
        );
    } else if !session.ready && session.screen == crate::menu::Screen::Racing {
        // Menus and lobbies show their own loading state.
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
/// Metres at which the nearby map's magnification halves.
const NEARBY_ZOOM: f64 = 100.;
/// Where the car sits in the nearby map, as a fraction of its height: low,
/// so more of the road ahead is in view.
const NEARBY_CAR_Y: f64 = 0.62;

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
            .step_by(2)
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

    /// Magnify the road around the car while smoothly compressing distant
    /// sections into the same view. No part of the circuit needs to be clipped.
    fn nearby(&self, point: (f32, f32), player: (f32, f32), yaw: f64) -> (f32, f32) {
        let dx = (point.0 - player.0) as f64 * self.span;
        let dz = (point.1 - player.1) as f64 * self.span;
        let left = dx * yaw.cos() - dz * yaw.sin();
        let ahead = dx * yaw.sin() + dz * yaw.cos();
        let radius = left.hypot(ahead);
        // ~300 m around the car fills most of the view; the rest of the lap
        // is compressed toward the rim instead of being cut off.
        // The car sits low, so the road ahead gets more of the view than the
        // road behind; every direction still stays inside the panel.
        let reach = 0.48 + 0.1 * ahead / radius.max(1e-9);
        let magnification = reach / (radius + NEARBY_ZOOM);
        (
            (0.5 - left * magnification) as f32,
            (NEARBY_CAR_Y - ahead * magnification) as f32,
        )
    }

    /// Distance from the car in metres, for fading the compressed far road.
    fn distance(&self, point: (f32, f32), player: (f32, f32)) -> f64 {
        ((point.0 - player.0) as f64).hypot((point.1 - player.1) as f64) * self.span
    }
}

fn draw_map(hud: &Hud, _style: &HudStyle, session: &Session, mode: MapMode, scale: f32) {
    let size = match mode {
        MapMode::FullCircuit => 240.,
        MapMode::Nearby => 330.,
    } * scale;
    let mx = hud.width() - size - 24. * scale;
    let my = 24. * scale;
    let inset = 24. * scale;
    let rect = egui::Rect::from_min_max(
        egui::pos2(mx + inset, my + inset),
        egui::pos2(mx + size - inset, my + size - 16. * scale),
    );
    let car = &session.race.entrants[session.player].car;
    let player = session.minimap.project(car.position.x, car.position.z);
    let map = |point| {
        let point = match mode {
            MapMode::FullCircuit => point,
            MapMode::Nearby => session.minimap.nearby(point, player, car.yaw),
        };
        (
            rect.left() + point.0 * rect.width(),
            rect.top() + point.1 * rect.height(),
        )
    };
    let points = &session.minimap.points;
    let ink = hud.finish.colors().ink;
    let at = |point| {
        let (x, y) = map(point);
        egui::pos2(x, y)
    };
    // The outline is a few long paths. One shape per four-metre segment was
    // some 3,500 shapes for egui to tessellate every frame, most of them a
    // fraction of a pixel long.
    let mut runs: Vec<(f32, Vec<egui::Pos2>)> = Vec::new();
    match mode {
        MapMode::FullCircuit => {
            let mut outline: Vec<_> = points.iter().step_by(4).map(|&point| at(point)).collect();
            outline.push(at(points[0]));
            runs.push((1., outline));
        }
        MapMode::Nearby => {
            // The compressed far road recedes, in eight steps of width and
            // opacity, so the road ahead reads first.
            let mut level = None;
            for (index, &point) in points.iter().enumerate() {
                let distance = session.minimap.distance(point, player);
                let near = 1. - ((distance - 250.) / 700.).clamp(0., 1.) * 0.75;
                let step = (near * 8.).round() as u8;
                let position = at(point);
                if level != Some(step) {
                    // Adjoining runs share their boundary point.
                    if let Some((_, run)) = runs.last_mut() {
                        run.push(position);
                    }
                    runs.push((step as f32 / 8., vec![position]));
                    level = Some(step);
                } else if step == 8 || index % 4 == 0 {
                    // Full detail around the car; distant road is a few pixels long.
                    runs.last_mut().expect("a run is open").1.push(position);
                }
            }
            if let Some((_, run)) = runs.last_mut() {
                run.push(at(points[0]));
            }
        }
    }
    for (near, run) in &runs {
        hud.painter.add(egui::Shape::line(
            run.clone(),
            egui::Stroke::new(
                (3. + 3. * near) * scale,
                Color::from_black_alpha((150. * near) as u8),
            ),
        ));
    }
    for (near, run) in runs {
        hud.painter.add(egui::Shape::line(
            run,
            egui::Stroke::new(
                (1.5 + 1.5 * near) * scale,
                ink.gamma_multiply(0.4 + 0.6 * near),
            ),
        ));
    }

    let start = map(points[0]);
    for row in 0..3 {
        for column in 0..3 {
            hud.fill_rect(
                start.0 + (column as f32 - 1.5) * 3. * scale,
                start.1 + (row as f32 - 1.5) * 3. * scale,
                3. * scale,
                3. * scale,
                if (row + column) % 2 == 0 {
                    WHITE
                } else {
                    PURPLE
                },
            );
        }
    }
    for (index, entrant) in session.race.entrants.iter().enumerate() {
        if index == session.player {
            continue;
        }
        let point = map(session
            .minimap
            .project(entrant.car.position.x, entrant.car.position.z));
        let clipped = Hud {
            painter: hud.painter.with_clip_rect(rect),
            size: hud.size,
            finish: hud.finish,
        };
        clipped.circle(point, 4.5 * scale, WELL);
        clipped.circle(point, 3. * scale, WHITE);
    }
    let me = map(player);
    let clipped = Hud {
        painter: hud.painter.with_clip_rect(rect),
        size: hud.size,
        finish: hud.finish,
    };
    clipped.circle(me, 7. * scale, WELL);
    clipped.circle(me, 5.5 * scale, ACCENT);
}

/// Short-lived banner messages.
#[derive(Default)]
pub struct Messages {
    active: Vec<(String, Color, f32)>,
    pub(crate) seen_events: usize,
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

/// Best-lap reference recorded at exact station crossings on physics ticks.
#[derive(Default)]
pub struct Delta {
    best: Vec<f64>,
    best_time: f64,
    lap: Vec<f64>,
    laps: u32,
    previous: Option<(f64, f64)>,
    average: telemetry::Average<72>,
    current: Option<f64>,
    last_tick: Option<u64>,
}

impl Delta {
    #[cfg(test)]
    pub(crate) fn is_reset(&self) -> bool {
        self.current.is_none() && self.previous.is_none()
    }
    fn record_to(&mut self, distance: f64, elapsed: f64, spacing: f64) {
        let (before, time) = self.previous.unwrap_or((0., 0.));
        if distance > before && distance - before < 50. {
            let first = (before / spacing).floor() as usize + 1;
            let last = (distance / spacing).floor() as usize;
            for station in first..=last.min(self.lap.len() - 1) {
                if self.lap[station].is_nan() {
                    let fraction = (station as f64 * spacing - before) / (distance - before);
                    self.lap[station] = time + fraction * (elapsed - time);
                }
            }
        }
        self.previous = Some((distance, elapsed));
    }

    fn update(&mut self, race: &racecraft_sim::Race, player: usize) {
        if self.last_tick == Some(race.tick) {
            return;
        }
        if self.last_tick.is_some_and(|tick| race.tick < tick) {
            *self = Self::default();
        }
        let timing = &race.entrants[player].timing;
        let count = race.track.stations.len();
        let spacing = race.track.length / count as f64;
        if self.lap.len() != count {
            *self = Self::default();
            self.lap = vec![f64::NAN; count];
            self.lap[0] = 0.;
        }
        self.last_tick = Some(race.tick);
        if timing.laps_completed != self.laps {
            if let Some(last) = timing.last_lap {
                self.record_to(race.track.length, last, spacing);
                if timing.laps_completed == self.laps + 1
                    && timing.last_lap == timing.best_lap
                    && self.lap.iter().all(|time| time.is_finite())
                {
                    self.best.clone_from(&self.lap);
                    self.best_time = last;
                }
            }
            self.lap.fill(f64::NAN);
            self.lap[0] = 0.;
            self.previous = None;
            self.average = Default::default();
            self.laps = timing.laps_completed;
        }
        self.current = None;
        if !matches!(race.phase, Phase::Racing | Phase::Finishing)
            || timing.finish_time.is_some()
            || timing.distance < 0.0
        {
            self.average = Default::default();
            return;
        }
        let elapsed = race.time - timing.lap_start;
        let distance = timing.distance.rem_euclid(race.track.length);
        self.record_to(distance, elapsed, spacing);
        let station = (distance / spacing).floor() as usize;
        if let Some(&before) = self.best.get(station) {
            let after = self
                .best
                .get(station + 1)
                .copied()
                .unwrap_or(self.best_time);
            let fraction = distance / spacing - station as f64;
            let delta = elapsed - (before + fraction * (after - before));
            self.average.push(delta);
            self.current = Some(self.average.value(delta));
        }
    }
}

pub fn format_time(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    let rest = seconds - minutes * 60.0;
    format!("{}:{:06.3}", minutes as u32, rest)
}

/// Tenths are readable while running; completed times retain milliseconds.
fn format_live_time(seconds: f64) -> String {
    let tenths = (seconds.max(0.) * 10.).floor() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

fn lap_clock(race: &racecraft_sim::Race, player: usize) -> String {
    let timing = &race.entrants[player].timing;
    if timing.finish_time.is_some() {
        return timing
            .last_lap
            .map(format_time)
            .unwrap_or_else(|| "-:--.---".into());
    }
    format_live_time(if matches!(race.phase, Phase::Countdown { .. }) {
        0.
    } else {
        race.time - timing.lap_start
    })
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn running_clock_uses_tenths_and_finish_freezes_the_exact_last_lap() {
        let mut session = crate::tests::session();
        session.race.phase = Phase::Racing;
        session.race.time = 65.987;
        assert_eq!(lap_clock(&session.race, 0), "1:05.9");
        session.race.entrants[0].timing.last_lap = Some(65.987);
        session.race.entrants[0].timing.finish_time = Some(65.987);
        session.race.time += 20.;
        assert_eq!(lap_clock(&session.race, 0), "1:05.987");
        assert_eq!(format_live_time(59.999), "0:59.9");
        assert_eq!(format_live_time(60.), "1:00.0");
    }

    #[test]
    fn an_identical_lap_has_no_station_sawtooth_including_the_finish_line() {
        let mut session = crate::tests::session();
        session.race.phase = Phase::Racing;
        let length = session.race.track.length;
        let lap_time = length / 40.;
        for tick in 0..(lap_time * 360. * 3.) as u64 {
            session.race.tick = tick;
            session.race.time = tick as f64 / 360.;
            let distance = session.race.time * 40.;
            let laps = (distance / length).floor() as u32;
            let timing = &mut session.race.entrants[0].timing;
            timing.distance = distance;
            timing.laps_completed = laps;
            timing.lap_start = laps as f64 * lap_time;
            if laps > 0 {
                timing.last_lap = Some(lap_time);
                timing.best_lap = Some(lap_time);
            }
            session.delta.update(&session.race, 0);
            if laps > 0 {
                let delta = session.delta.current.expect("best lap reference recorded");
                assert!(
                    delta.abs() < 1e-9,
                    "identical lap delta at {distance}: {delta}"
                );
            }
        }
    }
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
                let label = if best {
                    "PERSONAL BEST"
                } else {
                    "LAP COMPLETE"
                };
                session.messages.push(
                    format!("{label}   {}", format_time(time)),
                    if best { PURPLE } else { WHITE },
                    2.5,
                );
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

/// Width of the online ping column in the timing tower, in logical units.
const PING_COLUMN: f32 = 62.0;

/// Connection quality for a round-trip time, using the shared semantic colours.
pub fn ping_color(ms: u16) -> Color {
    match ms {
        0..=59 => GREEN,
        60..=119 => AMBER,
        _ => ACCENT,
    }
}

/// Signal bars plus the round-trip time, right-aligned in the tower's ping column.
fn ping_cell(
    hud: &mut Hud,
    style: &HudStyle,
    (x, y): (f32, f32),
    (w, h): (f32, f32),
    ms: u16,
    ink: Color,
    scale: f32,
) {
    // A hairline separates the ping column from the race gaps.
    hud.fill_rect(
        x,
        y + 5. * scale,
        scale.max(1.),
        h - 10. * scale,
        Color::from_white_alpha(28),
    );
    let color = ping_color(ms);
    let lit = match ms {
        0..=59 => 3,
        60..=119 => 2,
        _ => 1,
    };
    let bar_w = 4. * scale;
    let base = y + h * 0.5 + 6. * scale;
    for bar in 0..3 {
        let bar_h = (5. + 3.5 * bar as f32) * scale;
        hud.fill_rect(
            x + 8. * scale + bar as f32 * (bar_w + 2. * scale),
            base - bar_h,
            bar_w,
            bar_h,
            if bar < lit {
                color
            } else if ink.r() < 100 {
                // The player's inverted analog row is pale.
                Color::from_black_alpha(60)
            } else {
                Color::from_white_alpha(40)
            },
        );
    }
    readout(
        hud,
        style,
        15. * scale,
        (x + w - 12. * scale, y + h * 0.5),
        Align::Right,
        ink,
        &ms.min(999).to_string(),
    );
}

pub fn draw(hud: &mut Hud, style: &HudStyle, session: &mut Session, display: Settings, dt: f32) {
    if hud.width() <= 0.0 || hud.height() <= 0.0 {
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

    let scale = scale(hud.size);
    let colors = hud.finish.colors();
    let ink = colors.ink;
    let dim = colors.muted;
    let accent = colors.accent;
    let width = hud.width();
    let height = hud.height();
    let race = &session.race;
    let player = &race.entrants[session.player];
    let car = &player.car;
    let lap = (player.timing.laps_completed + 1).min(race.laps);
    // Broadcast timing strip; instrument chrome belongs to the physical dials.
    let tower_x = 24.0 * scale;
    // Online races add a fixed ping column to the right of the gaps.
    let online = !session.pings.is_empty();
    let ping_w = if online { PING_COLUMN * scale } else { 0.0 };
    let tower_w = 278.0 * scale + ping_w;
    let mut y = 24.0 * scale;
    let row = 28.0 * scale;
    // The LCD tower is opaque so unlit segments read as glass, not scenery.
    let finish = hud.finish;
    let smoke = move |alpha: u8| match finish {
        Finish::Digital => Color::from_rgb(6, 10, 11),
        Finish::Analog => Color::from_black_alpha(alpha),
    };
    hud.fill_rect(tower_x, y, tower_w, 35. * scale, smoke(210));
    let mid = y + 17.5 * scale;
    hud.text_mid(
        &style.bold,
        16. * scale,
        (tower_x + 12. * scale, mid),
        Align::Left,
        dim,
        "LAP",
    );
    readout(
        hud,
        style,
        16. * scale,
        (tower_x + 50. * scale, mid),
        Align::Left,
        ink,
        &format!("{lap}/{}", race.laps),
    );
    let team_position = format!(
        "{}  P{}",
        session
            .identities
            .get(session.player)
            .and_then(Option::as_ref)
            .map_or("", |id| id.team_short.as_str()),
        race.position_of(session.player)
    );
    // Reserve room for LAP and even a three-digit lap count. Fit using the
    // actual font, since equal character counts can have very different widths.
    // Keep the team label to the right of the widest four-digit lap readout.
    let team_width = (tower_w / scale - 152.).max(80.) * scale;
    let font_size = 18.2 * scale;
    let measured = hud.text_width(&style.regular, font_size, &team_position);
    let fitted = font_size * (team_width / measured.max(1.)).min(1.);
    let team_hud = Hud {
        painter: hud.painter.with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(tower_x + tower_w - 12. * scale - team_width, y),
            egui::pos2(tower_x + tower_w - 12. * scale, y + 35. * scale),
        )),
        size: hud.size,
        finish: hud.finish,
    };
    team_hud.text_mid(
        &style.regular,
        fitted,
        (tower_x + tower_w - 12. * scale, mid),
        Align::Right,
        dim,
        &team_position,
    );
    y += 38. * scale;
    let leader_distance = race.entrants[race.order()[0]].timing.distance;
    for (rank, &index) in race.order().iter().enumerate() {
        let entrant = &race.entrants[index];
        let mine = index == session.player;
        // Unlit segments would ghost on a light row, so the LCD marks the
        // player with a lit stripe instead of inverting the row.
        let digital = matches!(hud.finish, Finish::Digital);
        let inverted = mine && !digital;
        hud.fill_rect(
            tower_x,
            y,
            tower_w,
            row - scale,
            if inverted {
                Color::from_rgb(218, 222, 213)
            } else if mine {
                Color::from_rgb(30, 36, 34)
            } else {
                smoke(185)
            },
        );
        if mine && digital {
            hud.fill_rect(tower_x, y, 3. * scale, row - scale, accent);
        }
        let mine = inverted;
        let mid = y + (row - scale) * 0.5;
        let ink = if mine {
            Color::from_rgb(14, 17, 19)
        } else {
            ink
        };
        let dim = if mine {
            Color::from_rgb(25, 28, 30)
        } else {
            Color::from_rgb(224, 225, 216)
        };
        readout(
            hud,
            style,
            16.0 * scale,
            (tower_x + 34.0 * scale, mid),
            Align::Right,
            if mine { ink } else { dim },
            &(rank + 1).to_string(),
        );
        let identity = session.identities.get(index).and_then(Option::as_ref);
        let team = identity.map_or(rgba(0.9, 0.75, 0.1, 1.0), |identity| {
            let [r, g, b] = identity.primary;
            Color::from_rgb(r, g, b)
        });
        hud.fill_round(
            tower_x + 42.0 * scale,
            y + 7.0 * scale,
            4.0 * scale,
            row - 14.0 * scale,
            2.0 * scale,
            team,
        );
        hud.text_mid(
            &style.regular,
            20.0 * scale,
            (tower_x + 54.0 * scale, mid),
            Align::Left,
            if mine { ink } else { dim },
            &identity.map_or_else(|| entrant.name.to_uppercase(), |id| id.abbr.clone()),
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
                format!(
                    "+{:.1}",
                    session.telemetry.gaps.get(index).map_or_else(
                        || metres / entrant.car.speed().max(30.0),
                        |gap| gap.value(metres / entrant.car.speed().max(30.0)),
                    )
                )
            }
        };
        readout(
            hud,
            style,
            15.0 * scale,
            (tower_x + tower_w - ping_w - 14.0 * scale, mid),
            Align::Right,
            dim,
            &gap,
        );
        if online {
            if let Some(ms) = session.pings.get(index).copied().flatten() {
                ping_cell(
                    hud,
                    style,
                    (tower_x + tower_w - ping_w, y),
                    (ping_w, row - scale),
                    ms,
                    if mine { ink } else { dim },
                    scale,
                );
            }
        }
        y += row;
    }
    y += 8. * scale;
    let strip_h = timing::strip(hud, style, session, tower_x, y, tower_w, scale);
    timing::pass_notice(
        hud,
        style,
        session,
        tower_x,
        y + strip_h + 8. * scale,
        tower_w,
        scale,
    );

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
            let header_width =
                hud.text_width(&style.bold, 24. * scale, &corner.name.to_uppercase()) + 64. * scale;
            housing(
                hud,
                width * 0.5 - header_width * 0.5,
                24. * scale,
                header_width,
                46. * scale,
                scale,
            );
            hud.text(
                &style.bold,
                24.0 * scale,
                (width * 0.5, 29.0 * scale),
                Align::Center,
                ink,
                &corner.name.to_uppercase(),
            );
        }
    }
    let mut banner_y = 76.0 * scale;
    for (text, color, remaining) in &session.messages.active {
        let fade = remaining.min(0.4) / 0.4;
        let color = color.gamma_multiply(fade);
        let font_size = 18.0 * scale;
        let text_width = hud.text_width(&style.bold, font_size, text);
        let banner_w = text_width + 32.0 * scale;
        let banner_x = (width - banner_w) * 0.5;
        hud.fill_rect(
            banner_x,
            banner_y,
            banner_w,
            34.0 * scale,
            WELL.gamma_multiply(fade),
        );
        hud.fill_rect(banner_x, banner_y, 3.0 * scale, 34.0 * scale, color);
        hud.text_mid(
            &style.bold,
            font_size,
            (width * 0.5, banner_y + 17.0 * scale),
            Align::Center,
            color,
            text,
        );
        banner_y += 40.0 * scale;
    }

    instruments::draw(hud, style, session, display.layout, scale);

    instruments::tyres(hud, style, session, display.layout, scale);

    draw_map(hud, style, session, display.map, scale);

    // ---- Results: provisional as soon as the player takes the flag.
    let final_results = race.phase == Phase::Finished;
    if !session.pings.is_empty() && (final_results || player.timing.finish_time.is_some()) {
        let panel_w = 560.0 * scale;
        // Clear the timing tower, its control strip and the minimap.
        let left = (width * 0.5 - panel_w * 0.5).max(tower_x + tower_w + 16.0 * scale);
        let top = (height * 0.18).max(280.0 * scale);
        let rows = race.entrants.len() as f32;
        housing(hud, left, top, panel_w, (96.0 + rows * 34.0) * scale, scale);
        hud.fill_round(
            left + 24.0 * scale,
            top + 22.0 * scale,
            4.0 * scale,
            32.0 * scale,
            2.0 * scale,
            accent,
        );
        hud.text(
            &style.bold,
            36.0 * scale,
            (left + 38.0 * scale, top + 12.0 * scale),
            Align::Left,
            ink,
            if final_results {
                "RESULTS"
            } else {
                "PROVISIONAL"
            },
        );
        caption(
            hud,
            style,
            13.0 * scale,
            (left + 38.0 * scale, top + 52.0 * scale),
            Align::Left,
            if final_results {
                "ESC  MENU"
            } else {
                "OTHERS STILL FINISHING  ·  ESC  MENU"
            },
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
                    accent,
                );
            }
            let color = if mine { ink } else { dim };
            let mid = row_y + 15.0 * scale;
            hud.text_mid(
                &style.bold,
                24.0 * scale,
                (left + 62.0 * scale, mid),
                Align::Right,
                color,
                &format!("P{}", rank + 1),
            );
            hud.text_mid(
                &style.bold,
                24.0 * scale,
                (left + 78.0 * scale, mid),
                Align::Left,
                color,
                &entrant.name.to_uppercase(),
            );
            let best = entrant
                .timing
                .best_lap
                .map(format_time)
                .unwrap_or_else(|| "-:--.---".into());
            readout(
                hud,
                style,
                20.0 * scale,
                (left + panel_w - 28.0 * scale, mid),
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

    #[test]
    fn nearby_map_keeps_the_car_fixed_and_rotates_the_road_ahead_up() {
        let session = crate::tests::session();
        let map = &session.minimap;
        let x = session.race.entrants[0].car.position.x;
        let z = session.race.entrants[0].car.position.z;
        let player = map.project(x, z);
        for yaw in [
            0.,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
            -std::f64::consts::FRAC_PI_2,
        ] {
            let center = map.nearby(player, player, yaw);
            assert!((center.0 - 0.5).abs() < 1e-5 && (center.1 as f64 - NEARBY_CAR_Y).abs() < 1e-5);
            let ahead = map.project(x + yaw.sin() * 300., z + yaw.cos() * 300.);
            let ahead = map.nearby(ahead, player, yaw);
            assert!((ahead.0 - center.0).abs() < 1e-4 && ahead.1 < center.1);
            let left = map.project(x + yaw.cos() * 300., z - yaw.sin() * 300.);
            let left = map.nearby(left, player, yaw);
            assert!(left.0 < center.0 && (left.1 - center.1).abs() < 1e-4);
            for &point in &map.points {
                let focused = map.nearby(point, player, yaw);
                assert!((0. ..=1.).contains(&focused.0) && (0. ..=1.).contains(&focused.1));
            }
        }
    }
    #[test]
    fn focused_map_keeps_far_sections_visible_and_gives_nearby_road_more_space() {
        let session = crate::tests::session();
        let map = &session.minimap;
        for &player in map.points.iter().step_by(10) {
            for yaw in [0., 1., 3.] {
                for &point in &map.points {
                    let point = map.nearby(point, player, yaw);
                    assert!((0. ..=1.).contains(&point.0) && (0. ..=1.).contains(&point.1));
                }
            }
        }
        let p = (0.5, 0.5);
        let near = map.nearby((p.0, p.1 + (100. / map.span) as f32), p, 0.);
        let next = map.nearby((p.0, p.1 + (200. / map.span) as f32), p, 0.);
        let far = map.nearby((p.0, p.1 + (5000. / map.span) as f32), p, 0.);
        assert!(0.54 - near.1 > near.1 - next.1);
        assert!(far.1 > 0. && far.1 < next.1);
    }

    use racecraft_sim::car::spec::Compound;

    #[test]
    fn tyre_colors_follow_each_compounds_thermal_window() {
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
