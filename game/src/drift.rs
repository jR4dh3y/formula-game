//! Drift mode: a road car on the drift park, scored on angle and speed.
//!
//! Points flow while the car slides between `MIN_ANGLE` and `SPIN_ANGLE` above
//! `MIN_SPEED`, faster the bigger the angle and the higher the speed. A chain
//! of linked slides raises the multiplier; the chain banks into the total once
//! the car straightens for `LINK_GRACE` seconds, and is lost to a spin, a wall
//! or running onto the grass.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use racecraft_sim::{Assists, Surface};

pub const TRACK: &str = "driftpark";
pub const CAR: &str = "thar";
const MIN_ANGLE: f64 = 12.0;
const SPIN_ANGLE: f64 = 110.0;
const MIN_SPEED: f64 = 25.0 / 3.6;
const LINK_GRACE: f64 = 1.2;
/// Seconds of chained sliding per +0.5 on the multiplier, up to `MAX_MULTIPLIER`.
const MULTIPLIER_STEP: f64 = 2.0;
const MAX_MULTIPLIER: f64 = 5.0;
/// Collision impulse (N s) that ends a chain.
const WALL_IMPULSE: f64 = 1500.0;

/// Aids for drifting: no traction or stability control and no line guidance,
/// light steering smoothing so the wheel can reach full countersteer.
pub fn assists(base: Assists) -> Assists {
    Assists {
        steering: base.steering.min(0.2),
        traction: 0.0,
        stability: 0.0,
        ..base
    }
}

/// How a chain ended, shown briefly on the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Banked(u64),
    Spun,
    Wall,
    Grass,
}

#[derive(Resource, Default, Debug)]
pub struct DriftScore {
    /// Points in the current chain, before banking.
    pub chain: f64,
    pub multiplier: f64,
    /// Seconds the current chain has been sliding.
    pub chain_time: f64,
    /// Seconds since the car last slid.
    pub grace: f64,
    pub total: u64,
    pub best_chain: u64,
    /// Current slip angle, degrees (unsigned).
    pub angle: f64,
    pub drifting: bool,
    pub outcome: Option<(Outcome, f64)>,
}

impl DriftScore {
    fn end(&mut self, outcome: Outcome) {
        if let Outcome::Banked(points) = outcome {
            self.total += points;
            self.best_chain = self.best_chain.max(points);
        }
        if self.chain > 0.0 || outcome != Outcome::Banked(0) {
            self.outcome = Some((outcome, 2.0));
        }
        self.chain = 0.0;
        self.chain_time = 0.0;
        self.multiplier = 1.0;
    }

    /// Advances the score for one frame of `dt` seconds.
    pub fn advance(&mut self, slip: f64, speed: f64, on_grass: bool, impact: f64, dt: f64) {
        if let Some((_, left)) = &mut self.outcome {
            *left -= dt;
            if *left <= 0.0 {
                self.outcome = None;
            }
        }
        self.angle = slip;
        let chaining = self.chain > 0.0;
        if chaining && impact > WALL_IMPULSE {
            self.end(Outcome::Wall);
            return;
        }
        if chaining && slip > SPIN_ANGLE {
            self.end(Outcome::Spun);
            return;
        }
        if chaining && on_grass {
            self.end(Outcome::Grass);
            return;
        }
        self.drifting = speed > MIN_SPEED && (MIN_ANGLE..SPIN_ANGLE).contains(&slip) && !on_grass;
        if self.drifting {
            self.grace = 0.0;
            self.chain_time += dt;
            self.multiplier =
                (1.0 + (self.chain_time / MULTIPLIER_STEP).floor() * 0.5).min(MAX_MULTIPLIER);
            // Angle counts most up to about 45 degrees; speed in km/h.
            let angle = ((slip - MIN_ANGLE) / 33.0).clamp(0.0, 1.0);
            self.chain += (0.4 + angle) * speed * 3.6 * dt * self.multiplier;
        } else {
            self.grace += dt;
            if chaining && self.grace > LINK_GRACE {
                let points = self.chain.round() as u64;
                self.end(Outcome::Banked(points));
            }
        }
    }
}

/// Slip angle of the player's car in degrees and its speed, from the sim.
fn slip_of(car: &racecraft_sim::Car) -> f64 {
    let forward = car.velocity.dot(car.forward());
    let lateral = car.velocity.dot(car.left());
    lateral.atan2(forward).to_degrees().abs()
}

pub fn update(
    options: Res<crate::options::Options>,
    mut session: ResMut<crate::Session>,
    mut score: ResMut<DriftScore>,
    time: Res<Time>,
    mut last_race_time: Local<f64>,
) {
    if !options.drift() {
        return;
    }
    // A restart or a new session rewinds the race clock: start a fresh score.
    if session.race.time < *last_race_time - 0.5 {
        *score = DriftScore::default();
    }
    *last_race_time = session.race.time;
    let player = session.player;
    // Drifting aids, whatever the menu last applied.
    let aids = assists(session.race.entrants[player].car.assists);
    session.race.entrants[player].car.assists = aids;
    session.corner_braking = 0.0;
    session.show_racing_line = false;
    if !session.ready || session.paused || session.screen != crate::menu::Screen::Racing {
        return;
    }
    let car = &session.race.entrants[player].car;
    let on_grass = car
        .wheels
        .iter()
        .all(|wheel| wheel.surface == Surface::Grass);
    let (slip, speed) = (slip_of(car), car.speed());
    let impact = session.impact;
    score.advance(slip, speed, on_grass, impact, time.delta_secs_f64());
}

pub fn ui(
    options: Res<crate::options::Options>,
    session: Res<crate::Session>,
    score: Res<DriftScore>,
    mut contexts: EguiContexts,
) -> Result {
    // After lights out: during the countdown the start lights own the top.
    if !options.drift()
        || !session.ready
        || session.screen != crate::menu::Screen::Racing
        || matches!(session.race.phase, racecraft_sim::Phase::Countdown { .. })
    {
        return Ok(());
    }
    let context = contexts.ctx_mut()?;
    let amber = egui::Color32::from_rgb(255, 184, 28);
    let red = egui::Color32::from_rgb(235, 55, 55);
    egui::Area::new("drift-score".into())
        .anchor(egui::Align2::CENTER_TOP, [0.0, 16.0])
        .interactable(false)
        .show(context, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_unmultiplied(12, 13, 15, 215))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(22, 10))
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}", score.total))
                                .size(30.0)
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(
                            egui::RichText::new(format!("BEST CHAIN {}", score.best_chain))
                                .size(12.0)
                                .color(egui::Color32::from_gray(150)),
                        );
                        if score.chain > 0.0 {
                            ui.label(
                                egui::RichText::new(format!(
                                    "+{:.0}  x{:.1}",
                                    score.chain, score.multiplier
                                ))
                                .size(24.0)
                                .strong()
                                .color(amber),
                            );
                        }
                        // Angle bar: amber inside the scoring band, red near a spin.
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(220.0, 8.0), egui::Sense::hover());
                        let painter = ui.painter();
                        painter.rect_filled(rect, 3.0, egui::Color32::from_gray(45));
                        let share = (score.angle / SPIN_ANGLE).clamp(0.0, 1.0) as f32;
                        let colour = if score.angle > 75.0 {
                            red
                        } else if score.drifting {
                            amber
                        } else {
                            egui::Color32::from_gray(120)
                        };
                        let mut fill = rect;
                        fill.set_width(rect.width() * share);
                        painter.rect_filled(fill, 3.0, colour);
                        ui.label(
                            egui::RichText::new(format!("{:.0}°", score.angle))
                                .size(13.0)
                                .color(egui::Color32::from_gray(190)),
                        );
                        if let Some((outcome, _)) = score.outcome {
                            let (text, colour) = match outcome {
                                Outcome::Banked(points) => (format!("BANKED +{points}"), amber),
                                Outcome::Spun => ("SPUN: CHAIN LOST".to_string(), red),
                                Outcome::Wall => ("WALL: CHAIN LOST".to_string(), red),
                                Outcome::Grass => ("GRASS: CHAIN LOST".to_string(), red),
                            };
                            ui.label(egui::RichText::new(text).size(16.0).strong().color(colour));
                        }
                    });
                });
        });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    #[test]
    fn a_held_slide_scores_builds_the_multiplier_and_banks() {
        let mut score = DriftScore::default();
        // +0.5 every 2 s of linked sliding: x3 after 8 s.
        for _ in 0..(9.0 / DT) as usize {
            score.advance(35.0, 15.0, false, 0.0, DT);
        }
        assert_eq!(score.multiplier, 3.0, "{score:?}");
        let chain = score.chain;
        assert!(chain > 800.0, "{chain}");
        for _ in 0..(2.0 / DT) as usize {
            score.advance(2.0, 15.0, false, 0.0, DT);
        }
        assert_eq!(score.total, chain.round() as u64);
        assert_eq!(score.best_chain, score.total);
        assert_eq!(score.chain, 0.0);
    }

    #[test]
    fn spins_walls_and_grass_lose_the_chain() {
        for (slip, grass, impact, expected) in [
            (130.0, false, 0.0, Outcome::Spun),
            (35.0, false, 5000.0, Outcome::Wall),
            (35.0, true, 0.0, Outcome::Grass),
        ] {
            let mut score = DriftScore::default();
            for _ in 0..60 {
                score.advance(35.0, 15.0, false, 0.0, DT);
            }
            score.advance(slip, 15.0, grass, impact, DT);
            assert_eq!(score.total, 0);
            assert_eq!(score.outcome.map(|o| o.0), Some(expected));
        }
    }

    #[test]
    fn slow_or_straight_driving_scores_nothing() {
        let mut score = DriftScore::default();
        for _ in 0..300 {
            score.advance(35.0, 3.0, false, 0.0, DT);
            score.advance(5.0, 30.0, false, 0.0, DT);
        }
        assert_eq!(score.chain, 0.0);
        assert_eq!(score.total, 0);
    }

    #[test]
    fn drift_assists_drop_traction_and_stability_and_cap_steering() {
        let aids = assists(Assists::default());
        assert_eq!((aids.traction, aids.stability), (0.0, 0.0));
        assert!(aids.steering <= 0.2);
    }
}
