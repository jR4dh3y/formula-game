//! Presentation-only averages sampled at 360 Hz, shared by both egui contexts.

use racecraft_sim::{Phase, Race};

/// A bounded tick window, published at 20 Hz so the digits have time to read.
pub(super) struct Average<const N: usize> {
    samples: [f64; N],
    cursor: usize,
    count: usize,
    sum: f64,
    published: Option<f64>,
    ticks: usize,
}

impl<const N: usize> Default for Average<N> {
    fn default() -> Self {
        Self {
            samples: [0.; N],
            cursor: 0,
            count: 0,
            sum: 0.,
            published: None,
            ticks: 0,
        }
    }
}

impl<const N: usize> Average<N> {
    pub(super) fn push(&mut self, value: f64) {
        if !value.is_finite() {
            return;
        }
        self.sum += value - self.samples[self.cursor];
        self.samples[self.cursor] = value;
        self.cursor = (self.cursor + 1) % N;
        self.count = (self.count + 1).min(N);
        self.ticks += 1;
        if self.published.is_none() || self.ticks.is_multiple_of(18) {
            self.published = Some(self.sum / self.count as f64);
        }
    }

    pub(super) fn value(&self, fallback: f64) -> f64 {
        self.published.unwrap_or(fallback)
    }
}

#[derive(Default)]
struct Rival {
    ahead: bool,
    changed_ticks: u16,
}

pub(super) struct PassNotice {
    pub gained: bool,
    pub rival: usize,
    pub position: usize,
    pub until: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Split {
    pub time: f64,
    pub delta: Option<f64>,
    pub lap: u32,
    /// As fast as any car's best for this sector in the session.
    pub overall: bool,
}

/// Three minisectors per sector, timed on their own.
pub(super) const MINISECTORS: usize = 9;

/// How a minisector compared, in broadcast colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum MiniResult {
    /// Fastest of any car this session (purple).
    Overall,
    /// The player's best so far (green).
    Personal,
    /// Slower than the player's best (yellow).
    Slower,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MiniSplit {
    pub lap: u32,
    pub result: MiniResult,
}

#[derive(Default)]
pub struct Telemetry {
    pub(super) speed: Average<36>,
    pub(super) rpm: Average<36>,
    pub(super) throttle: Average<36>,
    pub(super) brake: Average<36>,
    pub(super) temperatures: [Average<90>; 2],
    pub(super) wear: [Average<90>; 2],
    pub(super) tow: Average<90>,
    pub(super) wake: Average<90>,
    pub(super) gaps: Vec<Average<90>>,
    rivals: Vec<Rival>,
    pub(super) pass: Option<PassNotice>,
    pub(super) splits: [Option<Split>; 3],
    pub(super) minis: [Option<MiniSplit>; MINISECTORS],
    mini_best: [Option<f64>; MINISECTORS],
    mini_overall: [Option<f64>; MINISECTORS],
    /// Per entrant: observed minisector and an optional valid boundary time.
    mini_clock: Vec<Option<(i64, Option<f64>)>>,
    sector: Option<(u32, usize, f64)>,
    best_sectors: [Option<f64>; 3],
    leader: Option<usize>,
    last_tick: Option<u64>,
}

impl Telemetry {
    pub(crate) fn reward_pass(&self) -> Option<u64> {
        self.pass.as_ref().filter(|p| p.gained).map(|p| p.until)
    }
    pub(crate) fn reward_sector_best(&self, sector: usize) -> bool {
        self.splits[sector].is_some_and(|s| s.delta.is_some_and(|d| d < 0.))
    }
    pub(crate) fn reward_values(&self) -> (f64, f64) {
        (self.speed.value(0.), self.tow.value(0.))
    }

    pub(super) fn recover(&mut self) {
        self.speed = Average::default();
        self.rpm = Average::default();
        self.throttle = Average::default();
        self.brake = Average::default();
        self.tow = Average::default();
        self.wake = Average::default();
        self.gaps.clear();
        self.rivals.clear();
        self.pass = None;
    }

    /// Pedal displays average the applied input over the same ticks as speed.
    pub(super) fn pedals(&mut self, input: racecraft_sim::CarInput) {
        self.throttle.push(input.throttle);
        self.brake.push(input.brake);
    }

    pub(super) fn update(&mut self, race: &Race, player: usize) {
        if self.last_tick == Some(race.tick) {
            return;
        }
        if self.last_tick.is_some_and(|tick| race.tick < tick) {
            *self = Self::default();
        }
        self.last_tick = Some(race.tick);
        self.update_splits(race, player);
        self.update_minis(race, player);
        let car = &race.entrants[player].car;
        self.speed.push(car.speed() * 3.6);
        self.rpm.push(car.powertrain.rpm);
        for axle in 0..2 {
            self.temperatures[axle].push(car.tyres[axle].temperature);
            self.wear[axle].push(car.tyres[axle].wear);
        }
        self.tow.push(car.aero.slipstream());
        self.wake.push(car.aero.dirty_air());
        let leader = race.order()[0];
        // A new leader changes every gap's reference, so discard the old window.
        if self.leader != Some(leader) || self.gaps.len() != race.entrants.len() {
            self.gaps = (0..race.entrants.len())
                .map(|_| Average::default())
                .collect();
            self.leader = Some(leader);
        }
        for (index, entrant) in race.entrants.iter().enumerate() {
            self.gaps[index].push(
                (race.entrants[leader].timing.distance - entrant.timing.distance).max(0.)
                    / entrant.car.speed().max(30.),
            );
        }
        self.update_pass(race, player);
    }

    fn update_splits(&mut self, race: &Race, player: usize) {
        let timing = &race.entrants[player].timing;
        let current = (timing.laps_completed, timing.sector, timing.sector_start);
        if let Some((lap, sector, started)) = self.sector {
            if (current.0 > lap || current.0 == lap && current.1 > sector) && timing.distance > 0. {
                // S3 disappears from sim's sector_times at the lap boundary.
                let time = race.time - started;
                let fastest = race
                    .entrants
                    .iter()
                    .filter_map(|entrant| entrant.timing.best_sectors[sector])
                    .fold(f64::INFINITY, f64::min);
                self.splits[sector] = Some(Split {
                    time,
                    delta: self.best_sectors[sector].map(|best| time - best),
                    lap,
                    overall: time <= fastest + 1e-9,
                });
            }
        }
        self.sector = Some(current);
        self.best_sectors = timing.best_sectors;
    }

    /// Times every car through each minisector, so each of the player's
    /// cells is coloured by its own time rather than the whole sector's.
    fn update_minis(&mut self, race: &Race, player: usize) {
        if self.mini_clock.len() != race.entrants.len() {
            self.mini_clock = vec![None; race.entrants.len()];
        }
        let length = race.track.length / MINISECTORS as f64;
        let racing = matches!(
            race.phase,
            racecraft_sim::Phase::Racing
                | racecraft_sim::Phase::Finishing
                | racecraft_sim::Phase::Finished
        );
        for (index, entrant) in race.entrants.iter().enumerate() {
            let distance = entrant.timing.distance;
            if !racing {
                self.mini_clock[index] = None;
                continue;
            }
            let count = (distance / length).floor() as i64;
            // Recovery ghosting identifies resets even within the same cell.
            // An unknown or skipped start must never time a partial cell.
            if entrant.ghost > 0. {
                self.mini_clock[index] = Some((count, None));
                continue;
            }
            let crossed = match self.mini_clock[index] {
                Some((previous, started)) if count == previous + 1 => {
                    self.mini_clock[index] = Some((count, Some(race.time)));
                    started.map(|started| (previous, started))
                }
                Some((previous, _)) if count == previous => continue,
                None if race.time == 0. && count == 0 => {
                    self.mini_clock[index] = Some((count, Some(race.time)));
                    None
                }
                _ => {
                    self.mini_clock[index] = Some((count, None));
                    None
                }
            };
            let Some((previous, started)) = crossed.filter(|(previous, _)| *previous >= 0) else {
                continue;
            };
            let mini = previous.rem_euclid(MINISECTORS as i64) as usize;
            let time = race.time - started;
            if index == player {
                let result = if self.mini_overall[mini].is_none_or(|best| time <= best) {
                    MiniResult::Overall
                } else if self.mini_best[mini].is_none_or(|best| time <= best) {
                    MiniResult::Personal
                } else {
                    MiniResult::Slower
                };
                self.minis[mini] = Some(MiniSplit {
                    lap: (previous / MINISECTORS as i64) as u32,
                    result,
                });
                self.mini_best[mini] =
                    Some(self.mini_best[mini].map_or(time, |best| best.min(time)));
            }
            self.mini_overall[mini] =
                Some(self.mini_overall[mini].map_or(time, |best| best.min(time)));
        }
    }

    fn update_pass(&mut self, race: &Race, player: usize) {
        if self
            .pass
            .as_ref()
            .is_some_and(|pass| race.tick >= pass.until)
        {
            self.pass = None;
        }
        if !matches!(race.phase, Phase::Racing | Phase::Finishing)
            || race.entrants[player].timing.finish_time.is_some()
        {
            self.pass = None;
            self.rivals.clear();
            return;
        }
        let position = race.position_of(player);
        if self.rivals.len() != race.entrants.len() {
            self.rivals = (0..race.entrants.len())
                .map(|index| Rival {
                    ahead: race.position_of(index) < position,
                    changed_ticks: 0,
                })
                .collect();
            return;
        }
        for (index, rival) in self.rivals.iter_mut().enumerate() {
            if index == player || race.entrants[index].timing.finish_time.is_some() {
                continue;
            }
            let ahead = race.position_of(index) < position;
            let clearance = (race.entrants[index].timing.distance
                - race.entrants[player].timing.distance)
                .abs();
            if ahead == rival.ahead || clearance < 1.0 {
                rival.changed_ticks = 0;
                continue;
            }
            rival.changed_ticks += 1;
            // 108 consecutive ticks = 300 ms with at least one metre clearance.
            if rival.changed_ticks >= 108 {
                rival.ahead = ahead;
                rival.changed_ticks = 0;
                self.pass = Some(PassNotice {
                    gained: !ahead,
                    rival: index,
                    position,
                    until: race.tick + 900,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use racecraft_sim::EntrantConfig;

    #[test]
    fn alternating_tick_noise_is_removed_but_a_real_change_arrives_within_150_ms() {
        let mut average = Average::<36>::default();
        for tick in 0..360 {
            average.push(if tick % 2 == 0 { 100. } else { 200. });
        }
        assert_eq!(average.value(0.), 150.);
        average.push(f64::NAN);
        assert_eq!(average.value(0.), 150.);
        for _ in 0..18 {
            average.push(250.);
        }
        assert_eq!(average.value(0.), 200.);
        for _ in 0..36 {
            average.push(250.);
        }
        assert_eq!(average.value(0.), 250.);
    }

    #[test]
    fn duplicate_render_reads_and_pause_do_not_add_samples_and_restart_discards_history() {
        let mut session = crate::tests::session();
        session.race.tick = 100;
        session.race.entrants[0].car.velocity = glam::DVec2::Y * 50.;
        session.sample_hud();
        session.race.entrants[0].car.velocity = glam::DVec2::ZERO;
        for _ in 0..144 {
            session.sample_hud();
        }
        assert_eq!(session.telemetry.speed.value(0.), 180.);
        session.race.tick = 0;
        session.sample_hud();
        assert_eq!(session.telemetry.speed.value(100.), 0.);
    }

    #[test]
    fn each_minisector_is_judged_on_its_own_time() {
        let mut session = crate::tests::session();
        session.race.phase = racecraft_sim::Phase::Racing;
        let length = session.race.track.length / MINISECTORS as f64;
        let mut clock = 0.;
        // Drive minisectors with the given durations, from `start` onward.
        let mut drive = |session: &mut crate::Session, start: usize, durations: &[f64]| {
            for (offset, duration) in durations.iter().enumerate() {
                let mini = start + offset;
                session.race.entrants[0].timing.distance = mini as f64 * length + 1.;
                session.race.time = clock;
                session.telemetry.update_minis(&session.race, 0);
                clock += duration;
            }
            session.race.entrants[0].timing.distance =
                (start + durations.len()) as f64 * length + 1.;
            session.race.time = clock;
            session.telemetry.update_minis(&session.race, 0);
        };
        // First lap: every minisector is the best so far (purple).
        drive(&mut session, 0, &[10.; MINISECTORS]);
        assert!(session.telemetry.minis.iter().all(|split| split
            .is_some_and(|split| split.result == MiniResult::Overall && split.lap == 0)));
        // Second lap: quicker through the first, slower through the second.
        drive(&mut session, MINISECTORS + 1, &[9., 11.]);
        let result = |index: usize| session.telemetry.minis[index].unwrap();
        assert_eq!(result(1).lap, 1);
        assert_eq!(result(1).result, MiniResult::Overall);
        assert_eq!(result(2).result, MiniResult::Slower);
        // The third minisector of lap one is untouched by lap two so far.
        assert_eq!(result(3).lap, 0);
    }

    #[test]
    fn recovery_and_skipped_boundaries_never_set_partial_bests_for_any_car() {
        for player in [0, 1] {
            for (from, to, ghost) in [(4., 2.5, 0.), (4., 4.5, 1.), (4., 7.5, 0.)] {
                let mut race = two_cars();
                race.phase = Phase::Racing;
                let length = race.track.length / MINISECTORS as f64;
                let mut display = Telemetry::default();
                let mut sample = |display: &mut Telemetry, time, cell, ghost| {
                    race.time = time;
                    for entrant in &mut race.entrants {
                        entrant.timing.distance = cell * length + 1.;
                        entrant.ghost = ghost;
                    }
                    display.update_minis(&race, player);
                };
                sample(&mut display, 10., from, 0.);
                sample(&mut display, 20., to, ghost);
                sample(&mut display, 21., to, 0.);
                sample(&mut display, 22., to.floor() + 1., 0.);
                assert!(display.mini_best.iter().all(Option::is_none));
                assert!(display.mini_overall.iter().all(Option::is_none));
                sample(&mut display, 32., to.floor() + 2., 0.);
                let index = (to.floor() as usize + 1) % MINISECTORS;
                assert_eq!(display.mini_best[index], Some(10.));
                assert_eq!(display.mini_overall[index], Some(10.));
            }
        }
    }

    #[test]
    fn final_minisector_is_recorded_when_the_race_finishes() {
        let mut session = crate::tests::session();
        session.race.phase = Phase::Racing;
        let length = session.race.track.length / MINISECTORS as f64;
        for (time, cell) in [(10., 7.), (20., 8.)] {
            session.race.time = time;
            session.race.entrants[0].timing.distance = cell * length + 1.;
            session.telemetry.update_minis(&session.race, 0);
        }
        session.race.phase = Phase::Finished;
        session.race.time = 30.;
        session.race.entrants[0].timing.distance = session.race.track.length;
        session.race.entrants[0].timing.finish_time = Some(30.);
        session.telemetry.update_minis(&session.race, 0);
        assert_eq!(session.telemetry.mini_best[8], Some(10.));
        assert_eq!(session.telemetry.minis[8].unwrap().lap, 0);
    }

    #[test]
    fn a_personal_improvement_behind_a_faster_rival_is_green() {
        let mut race = two_cars();
        race.phase = Phase::Racing;
        let length = race.track.length / MINISECTORS as f64;
        let mut display = Telemetry {
            mini_clock: vec![Some((0, Some(0.))); 2],
            ..Telemetry::default()
        };
        display.mini_best[0] = Some(10.);
        race.time = 8.;
        race.entrants[0].timing.distance = length - 1.;
        race.entrants[1].timing.distance = length + 1.;
        display.update_minis(&race, 0);
        race.time = 9.;
        race.entrants[0].timing.distance = length + 1.;
        display.update_minis(&race, 0);
        assert_eq!(display.minis[0].unwrap().result, MiniResult::Personal);
    }

    #[test]
    fn track_recovery_drops_motion_history_but_keeps_completed_splits() {
        let mut session = crate::tests::session();
        session.race.tick = 100;
        session.race.entrants[0].car.velocity = glam::DVec2::Y * 50.;
        session.sample_hud();
        session.telemetry.splits[0] = Some(Split {
            time: 31.125,
            delta: None,
            lap: 0,
            overall: false,
        });
        session.race.recover(0);
        session.recover_hud();
        assert_eq!(
            session
                .telemetry
                .speed
                .value(session.race.entrants[0].car.speed()),
            0.
        );
        assert!(session.telemetry.pass.is_none());
        assert_eq!(session.telemetry.splits[0].unwrap().time, 31.125);
    }

    #[test]
    fn completed_splits_stay_exact_and_sector_three_survives_the_new_lap() {
        let mut session = crate::tests::session();
        session.telemetry.update_splits(&session.race, 0);
        for (time, lap, sector) in [(31.125, 0, 1), (65.375, 0, 2), (100.5, 1, 0)] {
            session.race.time = time;
            let timing = &mut session.race.entrants[0].timing;
            timing.distance = 20.;
            timing.laps_completed = lap;
            timing.sector = sector;
            timing.sector_start = time;
            // The sim clears this on a new lap; the display must remember S3.
            timing.sector_times = [None; 3];
            session.telemetry.update_splits(&session.race, 0);
        }
        let splits = session.telemetry.splits.map(|split| split.unwrap().time);
        assert_eq!(splits, [31.125, 34.25, 35.125]);
        assert_eq!(session.telemetry.splits[2].unwrap().lap, 0);
        session.race.entrants[0].timing.best_sectors[0] = Some(31.125);
        session.telemetry.update_splits(&session.race, 0);
        session.race.time = 131.;
        session.race.entrants[0].timing.sector = 1;
        session.race.entrants[0].timing.sector_start = 131.;
        session.telemetry.update_splits(&session.race, 0);
        assert_eq!(session.telemetry.splits[0].unwrap().delta, Some(-0.625));
    }

    fn two_cars() -> Race {
        let session = crate::tests::session();
        let car = &session.race.entrants[0].car;
        Race::new(
            session.race.track.clone(),
            (0..2)
                .map(|index| EntrantConfig {
                    name: format!("Car {index}"),
                    spec: car.spec.clone(),
                    setup: car.setup,
                    assists: car.assists,
                    ai: None,
                    human_slot: index,
                    livery: [1.; 3],
                })
                .collect(),
            3,
            5,
        )
    }

    fn place(race: &mut Race, player_ahead: bool) {
        for (index, entrant) in race.entrants.iter_mut().enumerate() {
            let distance = if (index == 0) == player_ahead {
                60.
            } else {
                40.
            };
            entrant.timing.distance = distance;
            entrant.car.track_position.s = distance;
            let frame = race.track.frame_at(distance);
            entrant.car.position = frame.position;
            entrant.car.yaw = frame.tangent.x.atan2(frame.tangent.y);
        }
        race.phase = Phase::Racing;
        race.step(&[]);
    }

    #[test]
    fn pass_reversals_do_not_flash_and_a_sustained_pass_is_confirmed_once() {
        let mut race = two_cars();
        let mut display = Telemetry::default();
        place(&mut race, false);
        assert_eq!(race.position_of(0), 2);
        display.update(&race, 0);
        for ahead in [true, false, true, false] {
            place(&mut race, ahead);
            for _ in 0..90 {
                race.tick += 1;
                display.update(&race, 0);
            }
            assert!(display.pass.is_none());
        }
        place(&mut race, true);
        assert_eq!(race.position_of(0), 1);
        for _ in 0..108 {
            race.tick += 1;
            display.update(&race, 0);
        }
        let pass = display.pass.as_ref().unwrap();
        assert!(pass.gained);
        assert_eq!(pass.position, 1);
        let until = pass.until;
        for _ in 0..899 {
            race.tick += 1;
            display.update(&race, 0);
        }
        assert_eq!(display.pass.as_ref().unwrap().until, until);
        race.tick += 1;
        display.update(&race, 0);
        assert!(display.pass.is_none());
        place(&mut race, false);
        for _ in 0..108 {
            race.tick += 1;
            display.update(&race, 0);
        }
        assert!(!display.pass.as_ref().unwrap().gained);
        race.phase = Phase::Finished;
        race.tick += 1;
        display.update(&race, 0);
        assert!(display.pass.is_none());
    }
}
