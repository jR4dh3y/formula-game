//! Race HUD drawn through egui in Bevy.

mod instruments;
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
    frontend: Res<crate::menu::Frontend>,
) -> Result {
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

    /// A 600 m car-relative window with more space ahead than behind.
    fn nearby(&self, point: (f32, f32), player: (f32, f32), yaw: f64) -> (f32, f32) {
        let dx = (point.0 - player.0) as f64 * self.span;
        let dz = (point.1 - player.1) as f64 * self.span;
        let left = dx * yaw.cos() - dz * yaw.sin();
        let ahead = dx * yaw.sin() + dz * yaw.cos();
        ((0.5 - left / 600.) as f32, (0.72 - ahead / 600.) as f32)
    }
}

fn draw_map(hud: &Hud, _style: &HudStyle, session: &Session, mode: MapMode, scale: f32) {
    let size = 240. * scale;
    let mx = hud.width() - size - 24. * scale;
    let my = 24. * scale;
    let inset = 24. * scale;
    let rect = egui::Rect::from_min_max(
        egui::pos2(mx + inset, my + inset),
        egui::pos2(mx + size - inset, my + size - 16. * scale),
    );
    let clipped = Hud {
        painter: hud.painter.with_clip_rect(rect),
        size: hud.size,
        finish: hud.finish,
    };
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
    for index in 0..points.len() {
        let a = map(points[index]);
        let b = map(points[(index + 1) % points.len()]);
        if rect.intersects(
            egui::Rect::from_two_pos(egui::pos2(a.0, a.1), egui::pos2(b.0, b.1)).expand(3. * scale),
        ) {
            clipped.line(a, b, 6. * scale, Color::from_black_alpha(150));
            clipped.line(a, b, 3. * scale, hud.finish.colors().ink);
        }
    }
    let start = map(points[0]);
    for row in 0..3 {
        for column in 0..3 {
            clipped.fill_rect(
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
        clipped.circle(point, 4.5 * scale, WELL);
        clipped.circle(point, 3. * scale, WHITE);
    }
    let me = map(player);
    clipped.circle(me, 7. * scale, WELL);
    clipped.circle(me, 5.5 * scale, ACCENT);
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

pub fn draw(hud: &mut Hud, style: &HudStyle, session: &mut Session, display: Settings, dt: f32) {
    if hud.width() <= 0.0 || hud.height() <= 0.0 {
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
    let current = if matches!(race.phase, Phase::Countdown { .. }) {
        0.0
    } else {
        race.time - player.timing.lap_start
    };

    // Broadcast timing strip; instrument chrome belongs to the physical dials.
    let tower_x = 24.0 * scale;
    let tower_w = 278.0 * scale;
    let mut y = 24.0 * scale;
    let row = 28.0 * scale;
    // The LCD tower is opaque so unlit segments read as glass, not scenery.
    let smoke = |alpha: u8| match hud.finish {
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
    let team_width = 142. * scale;
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
                format!("+{:.1}", metres / entrant.car.speed().max(30.0))
            }
        };
        readout(
            hud,
            style,
            15.0 * scale,
            (tower_x + tower_w - 14.0 * scale, mid),
            Align::Right,
            dim,
            &gap,
        );
        y += row;
    }
    // Lap timer and split memory share one smoked timing strip.
    y += 6.0 * scale;
    hud.fill_rect(tower_x, y, tower_w, 94. * scale, smoke(200));
    hud.line(
        (tower_x + 14. * scale, y - 5. * scale),
        (tower_x + tower_w - 14. * scale, y - 5. * scale),
        scale,
        colors.edge,
    );
    let lines = [
        ("TIME", format_time(current.max(0.0)), ink),
        (
            "LAST",
            player
                .timing
                .last_lap
                .map(format_time)
                .unwrap_or_else(|| "-:--.---".into()),
            dim,
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
        let mid = y + 19.0 * scale;
        hud.text_mid(
            &style.bold,
            16.0 * scale,
            (tower_x + 16.0 * scale, mid),
            Align::Left,
            dim,
            label,
        );
        readout(
            hud,
            style,
            18.0 * scale,
            (tower_x + tower_w - 14.0 * scale, mid),
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
    if let Some(delta) = session.delta.current {
        let text = format!("{delta:+.3}");
        let box_w = 150.0 * scale;
        let box_x = width * 0.5 - box_w * 0.5;
        let color = if delta <= 0.0 { GREEN } else { ACCENT };
        hud.fill_round(box_x, 82.0 * scale, box_w, 40.0 * scale, 6.0 * scale, WELL);
        hud.stroke_round(
            box_x,
            82.0 * scale,
            box_w,
            40.0 * scale,
            6.0 * scale,
            2.0 * scale,
            color,
        );
        readout(
            hud,
            style,
            26.0 * scale,
            (width * 0.5, 102.0 * scale),
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
        hud.plate(
            egui::Rect::from_min_size(
                egui::pos2(banner_x, banner_y),
                egui::vec2(banner_w, 56. * scale),
            ),
            8. * scale,
            colors.top.gamma_multiply(fade),
            colors.bottom.gamma_multiply(fade),
            colors.edge.gamma_multiply(fade),
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

    instruments::draw(hud, style, session, display.layout, scale);

    instruments::tyres(hud, style, session, display.layout, scale);

    draw_map(hud, style, session, display.map, scale);

    // ---- Results
    if race.phase == Phase::Finished {
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
            assert!((center.0 - 0.5).abs() < 1e-5 && (center.1 - 0.72).abs() < 1e-5);
            let ahead = map.project(x + yaw.sin() * 300., z + yaw.cos() * 300.);
            let ahead = map.nearby(ahead, player, yaw);
            assert!((ahead.0 - 0.5).abs() < 1e-4 && (ahead.1 - 0.22).abs() < 1e-4);
            let left = map.project(x + yaw.cos() * 300., z - yaw.sin() * 300.);
            let left = map.nearby(left, player, yaw);
            assert!(left.0.abs() < 1e-4 && (left.1 - 0.72).abs() < 1e-4);
        }
    }
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
