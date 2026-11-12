//! Capture-only driving script, so effects can be photographed without a
//! person at the keyboard. `F1_SCRIPT="0:ai;9:0,1,0;10.5:1,0,0.4,1"` switches
//! at each race-clock second to the AI driver (`ai`) or to held
//! `throttle,brake,steer[,handbrake]` inputs. Unset in normal play.

use racecraft_sim::{AiDriver, CarInput, DriverProfile, Race};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    Ai,
    Hold([f64; 4]),
}

#[derive(bevy::prelude::Resource, Default)]
pub struct Script {
    steps: Vec<(f64, Step)>,
    pilot: Option<AiDriver>,
    ticks: u64,
}

impl Script {
    pub fn from_env() -> Option<Self> {
        Self::parse(&std::env::var("F1_SCRIPT").ok()?)
    }

    fn parse(text: &str) -> Option<Self> {
        let mut steps = Vec::new();
        for part in text.split(';').filter(|part| !part.trim().is_empty()) {
            let (time, action) = part.split_once(':')?;
            let step = if action.trim() == "ai" {
                Step::Ai
            } else {
                let mut values = [0.; 4];
                for (slot, value) in values.iter_mut().zip(action.split(',')) {
                    *slot = value.trim().parse().ok()?;
                }
                Step::Hold(values)
            };
            steps.push((time.trim().parse().ok()?, step));
        }
        Some(Self {
            steps,
            ..Default::default()
        })
    }

    /// The scripted command for this physics tick: `Ok` is a finished AI
    /// command, `Err` a raw input that still passes through the driving aids.
    pub fn input(&mut self, race: &Race, player: usize) -> Option<Result<CarInput, CarInput>> {
        let time = self.ticks as f64 / racecraft_sim::PHYSICS_RATE;
        self.ticks += 1;
        let step = self.steps.iter().rev().find(|(at, _)| *at <= time)?.1;
        let car = &race.entrants[player].car;
        match step {
            Step::Hold([throttle, brake, steer, handbrake]) => Some(Err(CarInput {
                throttle,
                brake,
                steer,
                handbrake: handbrake > 0.5,
                ..Default::default()
            })),
            Step::Ai => {
                let pilot = self.pilot.get_or_insert_with(|| {
                    AiDriver::new(
                        DriverProfile {
                            pace: 0.97,
                            aggression: 0.5,
                            consistency: 1.,
                        },
                        car,
                        &race.track,
                        player as u64 + 100,
                    )
                });
                let rivals = race.rivals_of(player);
                Some(Ok(pilot.drive(
                    car,
                    &race.track,
                    &rivals,
                    1. / racecraft_sim::PHYSICS_RATE,
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_parses_ai_and_held_inputs_and_rejects_garbage() {
        let script = Script::parse("0:ai; 9.5:0,1,0;12:1,0,-0.4,1").unwrap();
        assert_eq!(
            script.steps,
            vec![
                (0., Step::Ai),
                (9.5, Step::Hold([0., 1., 0., 0.])),
                (12., Step::Hold([1., 0., -0.4, 1.])),
            ]
        );
        assert!(Script::parse("soon:ai").is_none());
        assert!(Script::parse("1:fast").is_none());
    }
}
