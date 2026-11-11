//! Offline driving rewards, bounded rewind history and repeat races.
use crate::{
    menu::{Frontend, Screen},
    options::Options,
    Session,
};
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use racecraft_sim::{race::RaceEvent, Phase, Race};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub xp: u64,
    pub best: BTreeMap<String, f64>,
}
impl Progress {
    fn save(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
    fn level(&self) -> u64 {
        1 + self.xp / 2000
    }
}
#[derive(Clone, Default)]
pub struct Score {
    pub banked: u64,
    seen_events: usize,
    chain: u64,
    feats: u64,
    last: u64,
    pub label: String,
    clean_sector: bool,
    clean_lap: bool,
    pub clean_laps: u32,
    sector: usize,
    best: Option<f64>,
    pass: u64,
    tow: u32,
    close: u32,
    speed: u32,
    start: bool,
    /// Latest race tick seen, and the tick the label was set on.
    now: u64,
    shown: u64,
}
/// Ticks a banked or lost notice stays up (2.5 s at 360 Hz).
const NOTICE_TICKS: u64 = 900;
impl Score {
    fn award(&mut self, label: &str, points: u64, tick: u64) {
        self.chain += points;
        self.feats += 1;
        self.last = tick;
        self.label = label.into();
        self.shown = tick;
    }
    fn bank(&mut self) {
        let earned = self.chain * self.feats.clamp(1, 5);
        if earned > 0 {
            self.label = format!("BANKED +{earned}");
            self.shown = self.now;
        }
        self.banked += earned;
        self.chain = 0;
        self.feats = 0;
    }
    fn lose(&mut self) {
        // Only a chain that held points is worth a notice.
        if self.chain > 0 {
            self.label = "CHAIN LOST".into();
            self.shown = self.now;
        }
        self.chain = 0;
        self.feats = 0;
        self.clean_sector = false;
        self.clean_lap = false;
    }
    /// The label to show now: the live chain's feat, or a brief notice.
    fn notice(&self) -> Option<&str> {
        (!self.label.is_empty()
            && (self.chain > 0 || self.now.saturating_sub(self.shown) < NOTICE_TICKS))
            .then_some(self.label.as_str())
    }
    fn tick(&mut self, session: &Session) {
        let race = &session.race;
        let e = &race.entrants[session.player];
        if !matches!(
            race.phase,
            Phase::Racing | Phase::Finishing | Phase::Finished
        ) {
            return;
        }
        let tick = race.tick;
        self.now = tick;
        let dirty = e.car.impact > 0.
            || e.ghost > 0.
            || e.car.track_position.lateral.abs()
                > race.track.stations[e.car.track_position.station].half_width;
        if dirty {
            self.lose();
            self.pass = session.telemetry.reward_pass().unwrap_or(self.pass);
            self.tow = 0;
            self.close = 0;
        }
        if e.timing.sector != self.sector {
            if self.clean_sector {
                self.award("CLEAN SECTOR", 100, tick);
            }
            if session.telemetry.reward_sector_best(self.sector) {
                self.award("PERSONAL SECTOR", 150, tick);
            }
            self.sector = e.timing.sector;
            self.clean_sector = !dirty;
        }
        for event in &race.events[self.seen_events.min(race.events.len())..] {
            if let RaceEvent::LapCompleted { entrant, time, .. } = event {
                if *entrant == session.player {
                    if self.clean_lap {
                        self.clean_laps += 1;
                        self.award("CLEAN LAP", 300, tick);
                    }
                    if self.best.is_none_or(|best| *time < best) {
                        self.award("PERSONAL LAP", 400, tick);
                        self.best = Some(*time);
                    }
                    self.clean_lap = !dirty;
                }
            }
        }
        self.seen_events = race.events.len();
        if !dirty {
            if let Some(until) = session
                .telemetry
                .reward_pass()
                .filter(|until| *until != self.pass)
            {
                self.pass = until;
                self.award("CLEAN PASS", 200, tick);
            }
            let (speed, tow) = session.telemetry.reward_values();
            self.tow = if tow > 0.05 && speed > 100. {
                self.tow + 1
            } else {
                0
            };
            if self.tow >= 1080 {
                self.tow = 0;
                self.award("IN THE TOW", 100, tick);
            }
            let close = race.entrants.iter().enumerate().any(|(i, rival)| {
                i != session.player
                    && rival.car.position.distance(e.car.position) < 12.
                    && speed > 100.
            });
            self.close = if close { self.close + 1 } else { 0 };
            if self.close >= 1800 {
                self.close = 0;
                self.award("CLOSE RACING", 100, tick);
            }
            let band = (speed / 50.) as u32;
            if band >= 4 && band > self.speed {
                self.speed = band;
                self.award("SPEED MARK", 100, tick);
            }
            if !self.start && race.time >= 8. {
                self.start = true;
                if speed > 100. && self.clean_lap {
                    self.award("GOOD START", 150, tick);
                }
            }
        }
        if self.chain > 0 && tick.saturating_sub(self.last) >= 1440 {
            self.bank();
        }
    }
}
struct Snapshot {
    race: Race,
    score: Score,
}
#[derive(Resource, Default)]
pub struct Run {
    last_tick: u64,
    initial: Option<Race>,
    history: VecDeque<Snapshot>,
    pub score: Score,
    pub progress: Progress,
    path: Option<PathBuf>,
    loaded: bool,
    resume: f32,
    held: f32,
    settled: bool,
    earned: u64,
    breakdown: Vec<(String, u64)>,
    error: Option<String>,
    pub restart: bool,
    pub next: bool,
}
fn offline(options: &Options) -> bool {
    options.host.is_none() && options.join.is_none() && options.benchmark.is_none()
}
impl Run {
    pub(crate) fn summary(&self) -> bool {
        self.settled
    }
    pub(crate) fn resuming(&self) -> bool {
        self.resume > 0.
    }
    pub fn sample(&mut self, session: &Session, options: &Options, rewind: bool) {
        if !offline(options) {
            return;
        }
        if self.initial.is_none() || session.race.tick <= self.last_tick {
            self.initial = Some(session.race.clone());
            self.score = Score {
                clean_sector: true,
                clean_lap: true,
                ..default()
            };
            self.history.clear();
            self.settled = false;
        }
        self.last_tick = session.race.tick;
        self.score.tick(session);
        if rewind
            && options.frames.is_none()
            && session.race.tick.is_multiple_of(180)
            && !self.settled
        {
            self.history.push_back(Snapshot {
                race: session.race.clone(),
                score: self.score.clone(),
            });
            while self.history.len() > 13 {
                self.history.pop_front();
            }
        }
    }
    fn load(&mut self, options: &Options) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if cfg!(test)
            || options.quick_race
            || options.frames.is_some()
            || !offline(options)
            || std::env::var_os("F1_SETTINGS").is_some()
        {
            return;
        }
        self.path = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .map(|p| p.join("f1-racecraft/progress.json"));
        if let Some(path) = &self.path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(progress) => self.progress = progress,
                    Err(e) => {
                        self.error = Some(format!("Progress unreadable: {e}"));
                        self.path = None;
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    self.error = Some(format!("Progress unavailable: {e}"));
                    self.path = None;
                }
            }
        }
    }
}
fn jump(session: &mut Session, race: Race) {
    session.race = race;
    session.poses = crate::pose::Poses::new(&session.race);
    let mut camera = crate::camera::ChaseCamera::new(&session.race, session.player);
    camera.select(session.camera.mode);
    camera.configure(session.camera.tuning.clone());
    camera.set_cockpit_eye(session.rig.cockpit_eye);
    session.camera = camera;
    session.guidance = crate::driving::Guidance::new(&session.race, session.player);
    session.controls = default();
    session.messages = default();
    session.delta = default();
    session.telemetry = default();
    session.impact = 0.;
    session.pending_impact = 0.;
    session.screen = Screen::Racing;
}
pub fn update(
    mut run: ResMut<Run>,
    mut session: ResMut<Session>,
    options: Res<Options>,
    frontend: Res<Frontend>,
    input: (Res<ButtonInput<KeyCode>>, Query<&Gamepad>, Res<Time>),
    online: Option<Res<crate::online::Online>>,
) {
    if !offline(&options) || online.as_ref().is_some_and(|o| o.active()) {
        return;
    }
    let (keys, pads, time) = input;
    run.load(&options);
    if !session.ready {
        return;
    }
    if !frontend.rewind {
        run.history.clear();
    }
    if run.resume > 0. {
        run.resume = (run.resume - time.delta_secs()).max(0.);
        session.paused = run.resume > 0. || session.screen != Screen::Racing;
    }
    let available = options.frames.is_none();
    if available && session.screen == Screen::Racing {
        let rewind = keys.just_pressed(KeyCode::KeyZ)
            || pads
                .iter()
                .any(|p| p.just_pressed(GamepadButton::RightThumb));
        if rewind
            && frontend.rewind
            && run.resume == 0.
            && session.race.entrants[session.player]
                .timing
                .finish_time
                .is_none()
        {
            if let Some(snapshot) = run.history.pop_front() {
                jump(&mut session, snapshot.race);
                run.last_tick = session.race.tick;
                run.score = snapshot.score;
                run.score.lose();
                run.history.clear();
                run.resume = 2.;
                session.paused = true;
            }
        }
        let held =
            keys.pressed(KeyCode::KeyT) || pads.iter().any(|p| p.pressed(GamepadButton::LeftThumb));
        run.held = if held {
            run.held + time.delta_secs()
        } else {
            0.
        };
        if run.held >= 1.5 {
            run.restart = true;
            run.held = -1000.;
        }
    }
    if available && (run.restart || run.next) {
        if let Some(mut race) = run.initial.clone() {
            if run.next {
                race.laps += 1;
            }
            race.entrants[session.player].car.assists = frontend.aids;
            jump(&mut session, race.clone());
            run.last_tick = session.race.tick;
            session.paused = false;
            run.initial = Some(race);
            run.history.clear();
            run.score = Score {
                clean_sector: true,
                clean_lap: true,
                ..default()
            };
            run.settled = false;
            run.resume = 0.;
        }
        run.restart = false;
        run.next = false;
    }
    if !run.settled
        && session.race.entrants[session.player]
            .timing
            .finish_time
            .is_some()
    {
        run.score.bank();
        run.settled = true;
        let position = session.race.position_of(session.player);
        let gained = options.grid_slot.saturating_sub(position) as u64;
        let aids = session.race.entrants[session.player].car.assists;
        let bonus =
            ((4. - aids.steering - aids.braking - aids.traction - aids.stability) * 100.) as u64;
        run.breakdown = vec![
            (
                format!("Finish P{position}"),
                500 + (session.race.entrants.len() - position) as u64 * 50,
            ),
            (format!("Positions gained: {gained}"), gained * 100),
            (
                format!("Clean laps: {}", run.score.clean_laps),
                run.score.clean_laps as u64 * 100,
            ),
            ("Banked driving points".into(), run.score.banked),
            ("Aid bonus".into(), bonus),
            ("Race distance".into(), session.race.laps as u64 * 100),
        ];
        run.earned = run.breakdown.iter().map(|(_, n)| n).sum();
        run.progress.xp += run.earned;
        if let Some(best) = session.race.entrants[session.player].timing.best_lap {
            let key = format!("{}/{}", options.track, options.car);
            let old = run.progress.best.entry(key).or_insert(best);
            *old = old.min(best);
        }
        if let Some(path) = &run.path {
            if let Err(e) = run.progress.save(path) {
                run.error = Some(format!("Progress not saved: {e}"));
            }
        }
    }
}
pub fn ui(
    mut contexts: EguiContexts,
    mut run: ResMut<Run>,
    mut session: ResMut<Session>,
    options: Res<Options>,
    frontend: Res<Frontend>,
    online: Option<Res<crate::online::Online>>,
) -> Result {
    if !offline(&options) || online.as_ref().is_some_and(|o| o.active()) {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    if session.screen == Screen::Main && frontend.launched {
        egui::Window::new("Driver record")
            .anchor(egui::Align2::RIGHT_TOP, [-16., 70.])
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Level {}  •  {} / 2000 XP",
                    run.progress.level(),
                    run.progress.xp % 2000
                ));
                if let Some(best) = run
                    .progress
                    .best
                    .get(&format!("{}/{}", options.track, options.car))
                {
                    ui.label(format!("Personal lap {}", crate::hud::format_time(*best)));
                }
            });
    }
    if session.screen == Screen::Paused {
        egui::Window::new("Go again")
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16., -100.])
            .show(ctx, |ui| {
                if ui.button("Restart race").clicked() {
                    run.restart = true;
                }
                if ui.button("Next: one more lap").clicked() {
                    run.next = true;
                }
            });
    }
    if session.screen != Screen::Racing || !session.ready {
        return Ok(());
    }
    if run.settled {
        egui::Window::new("RACE COMPLETE")
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0., 0.])
            .resizable(false)
            .default_width(350.)
            .show(ctx, |ui| {
                for (label, points) in &run.breakdown {
                    ui.label(format!("{label}    +{points} XP"));
                }
                if let Some(best) = session.race.entrants[session.player].timing.best_lap {
                    ui.label(format!("Best lap {}", crate::hud::format_time(best)));
                }
                ui.separator();
                ui.label(if session.race.phase == Phase::Finished {
                    "RESULTS"
                } else {
                    "PROVISIONAL: others still finishing"
                });
                egui::ScrollArea::vertical()
                    .max_height(240.)
                    .show(ui, |ui| {
                        for (rank, &index) in session.race.order().iter().enumerate() {
                            let entrant = &session.race.entrants[index];
                            let best = entrant
                                .timing
                                .best_lap
                                .map(crate::hud::format_time)
                                .unwrap_or_else(|| "-:--.---".into());
                            ui.label(format!("P{}  {}  {}", rank + 1, entrant.name, best));
                        }
                    });
                ui.separator();
                ui.heading(format!(
                    "+{} XP • Level {}",
                    run.earned,
                    run.progress.level()
                ));
                ui.add(
                    egui::ProgressBar::new((run.progress.xp % 2000) as f32 / 2000.)
                        .text(format!("{} / 2000 XP", run.progress.xp % 2000)),
                );
                if let Some(error) = &run.error {
                    ui.label(error);
                }
                if ui.button("Race again").clicked() {
                    run.restart = true;
                }
                if ui.button("Next: one more lap").clicked() {
                    run.next = true;
                }
                if ui.button("Menu").clicked() {
                    session.screen = Screen::Main;
                    session.paused = true;
                }
            });
    } else if run.score.notice().is_some() || run.held > 0. || run.resume > 0. {
        egui::Area::new("driving-rewards".into())
            .anchor(
                egui::Align2::RIGHT_TOP,
                [
                    -16.,
                    (match frontend.display.map {
                        crate::hud::MapMode::FullCircuit => 272.,
                        crate::hud::MapMode::Nearby => 362.,
                    }) * crate::hud::scale(ctx.content_rect().size()),
                ],
            )
            .show(ctx, |ui| {
                crate::menu::housing().inner_margin(10).show(ui, |ui| {
                    ui.set_width(190.);
                    if let Some(notice) = run.score.notice() {
                        ui.label(egui::RichText::new(notice).strong());
                        if run.score.chain > 0 {
                            ui.label(format!(
                                "+{} ×{}",
                                run.score.chain,
                                run.score.feats.clamp(1, 5)
                            ));
                        } else {
                            ui.label(format!("{} points", run.score.banked));
                        }
                    }
                    if run.held > 0. {
                        ui.add(egui::ProgressBar::new(run.held / 1.5).text("Hold to restart"));
                    }
                    if run.resume > 0. {
                        ui.label(format!("Resume in {:.0}", run.resume.ceil()));
                    }
                });
            });
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chain_banks_and_contact_loses_only_pending() {
        let mut s = Score::default();
        s.award("a", 100, 0);
        s.award("b", 200, 1);
        s.bank();
        assert_eq!(s.banked, 600);
        s.award("c", 500, 2);
        s.lose();
        s.bank();
        assert_eq!(s.banked, 600);
    }
    #[test]
    fn progression_round_trip() {
        let mut p = Progress {
            xp: 4300,
            ..default()
        };
        p.best.insert("spa/rb6".into(), 100.);
        let path = std::env::temp_dir().join(format!(
            "racecraft-progress-test-{}.json",
            std::process::id()
        ));
        p.save(&path).unwrap();
        let restored: Progress = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(restored.level(), 3);
        assert_eq!(restored.best["spa/rb6"], 100.);
    }
}

#[cfg(test)]
mod rewind_tests {
    use super::*;
    #[test]
    fn restore_resimulation_and_hud_reset() {
        let mut session = crate::tests::session();
        let initial = session.race.clone();
        let input = racecraft_sim::CarInput {
            throttle: 0.8,
            ..default()
        };
        for _ in 0..720 {
            session.race.step(&[input]);
            session.sample_hud();
        }
        let expected = session.race.entrants[0].car.position;
        let time = session.race.time;
        jump(&mut session, initial);
        assert!(session.delta.is_reset());
        assert!(session.telemetry.reward_pass().is_none());
        for _ in 0..720 {
            session.race.step(&[input]);
            session.sample_hud();
        }
        assert_eq!(session.race.entrants[0].car.position, expected);
        assert_eq!(session.race.time, time);
    }
    #[test]
    fn multiplier_caps_at_five() {
        let mut score = Score::default();
        for tick in 0..10 {
            score.award("pass", 100, tick);
        }
        score.bank();
        assert_eq!(score.banked, 5000);
    }
}

#[cfg(test)]
mod proof {
    use super::*;
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};
    #[test]
    #[ignore = "requires Vulkan and converted assets; synthetic presentation states"]
    fn rendered_loop_states() {
        let size = std::env::var("F1_TEST_SIZE").unwrap_or_else(|_| "1600x900".into());
        let options = Options::parse(
            [
                "--quick-race",
                "--mute",
                "--assets",
                concat!(env!("CARGO_MANIFEST_DIR"), "/../assets"),
                "--size",
                &size,
                "--frames",
                "10000",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();
        let mut app = crate::build_app(options).unwrap();
        app.world_mut().resource_mut::<Frontend>().video.texture =
            crate::graphics::TextureDetail::Half;
        app.add_systems(Update, fixture.after(update));
        assert!(app.run().is_success());
    }
    fn fixture(
        mut commands: Commands,
        mut run: ResMut<Run>,
        mut session: ResMut<Session>,
        mut frame: Local<u32>,
        mut exit: MessageWriter<AppExit>,
    ) {
        if !session.ready {
            return;
        }
        session.paused = true;
        *frame += 1;
        if *frame == 1 {
            session.race.phase = Phase::Racing;
            run.score.award("CLEAN PASS", 200, 0);
            run.score.award("IN THE TOW", 100, 0);
        }
        if (40..80).contains(&*frame) {
            run.resume = 2.;
        }
        if *frame == 80 {
            let player = session.player;
            session.race.entrants[player].timing.best_lap = Some(108.432);
            run.resume = 0.;
            run.settled = true;
            run.score.bank();
            run.earned = 1800;
            run.progress.xp = 2300;
            run.breakdown = vec![
                ("Finish P3".into(), 750),
                ("Positions gained: 4".into(), 400),
                ("Clean laps: 1".into(), 100),
                ("Banked driving points".into(), 300),
                ("Aid bonus".into(), 150),
                ("Race distance".into(), 100),
            ];
        }
        let name = match *frame {
            30 => "reward",
            50 => "rewind",
            90 => "summary",
            120 => {
                exit.write(AppExit::Success);
                return;
            }
            _ => return,
        };
        let dir = std::env::var("F1_TEST_OUTPUT").unwrap_or_else(|_| "/tmp/f1-loop-proof".into());
        std::fs::create_dir_all(&dir).unwrap();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("{dir}/{name}.png")));
    }
}

#[cfg(test)]
mod scoring_tests {
    use super::*;
    #[test]
    fn lap_events_are_consumed_once_and_cursor_rewinds_with_score() {
        let mut session = crate::tests::session();
        session.race.phase = Phase::Racing;
        session.race.events.push(RaceEvent::LapCompleted {
            entrant: session.player,
            lap: 1,
            time: 100.,
        });
        let mut score = Score {
            clean_lap: true,
            ..default()
        };
        score.tick(&session);
        let snapshot = score.clone();
        let chain = score.chain;
        score.tick(&session);
        assert_eq!(score.clean_laps, 1);
        assert_eq!(score.chain, chain);
        score = snapshot;
        session.race.events.push(RaceEvent::LapCompleted {
            entrant: session.player,
            lap: 2,
            time: 101.,
        });
        score.tick(&session);
        assert_eq!(score.clean_laps, 2);
    }
    #[test]
    fn quiet_banks_and_off_track_breaks_clean_credit() {
        let mut session = crate::tests::session();
        session.race.phase = Phase::Racing;
        session.race.tick = 1441;
        let mut score = Score {
            clean_lap: true,
            clean_sector: true,
            ..default()
        };
        score.award("pass", 200, 1);
        score.tick(&session);
        assert_eq!(score.banked, 200);
        score.award("tow", 100, 1441);
        session.race.entrants[session.player]
            .car
            .track_position
            .lateral = 100.;
        score.tick(&session);
        assert_eq!(score.chain, 0);
        assert!(!score.clean_lap);
    }
    #[test]
    fn snapshot_history_is_bounded_and_automation_excluded() {
        let mut session = crate::tests::session();
        let options = Options::parse(["--quick-race"].into_iter().map(str::to_owned)).unwrap();
        let mut run = Run::default();
        for i in 1..30 {
            session.race.tick = i * 180;
            run.sample(&session, &options, true);
        }
        assert_eq!(run.history.len(), 13);
        let automated = Options::parse(["--frames", "100"].into_iter().map(str::to_owned)).unwrap();
        let mut capture = Run::default();
        capture.sample(&session, &automated, true);
        assert!(capture.history.is_empty());
    }
}
