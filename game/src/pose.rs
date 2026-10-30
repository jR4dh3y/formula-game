//! Render poses blended between physics ticks.
//!
//! The simulation advances in fixed 1/360 s ticks while frames arrive at the
//! display rate, so a frame runs an uneven number of ticks. Drawing the latest
//! tick directly makes the car judder against the smoothly moving camera;
//! drawing between the last two ticks keeps motion even.

use glam::{DQuat, DVec3, EulerRot};
use racecraft_sim::{Car, Race};
use std::f64::consts::{PI, TAU};

/// Anything further than this in one tick is a recovery, not driving.
const TELEPORT: f64 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Model origin on the ground between the axles.
    pub origin: DVec3,
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub steer: f64,
    pub wheels: [f64; 4],
    /// Interpolated contact heights, independent of sprung chassis movement.
    pub ground: [f64; 4],
}

fn angle_lerp(from: f64, to: f64, t: f64) -> f64 {
    from + ((to - from + PI).rem_euclid(TAU) - PI) * t
}

impl Pose {
    pub fn of(car: &Car) -> Self {
        Self {
            origin: car.model_origin(),
            yaw: car.yaw,
            pitch: car.pitch,
            roll: car.roll,
            steer: car.steer_angle,
            wheels: std::array::from_fn(|index| car.wheels[index].rotation),
            ground: std::array::from_fn(|index| car.wheels[index].ground_height),
        }
    }

    fn resample_ground(
        &mut self,
        collision: &racecraft_sim::track::CollisionWorld,
        wheels: &[racecraft_sim::WheelSpec; 4],
    ) {
        let rotation = DQuat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, self.roll);
        for (index, wheel) in wheels.iter().enumerate() {
            let position = self.origin + rotation * wheel.position;
            if let Some(hit) =
                collision.ground(DVec3::new(position.x, self.ground[index] + 1.5, position.z))
            {
                self.ground[index] = hit.height;
            }
        }
    }

    pub fn lerp(&self, next: &Self, t: f64) -> Self {
        Self {
            origin: self.origin.lerp(next.origin, t),
            yaw: angle_lerp(self.yaw, next.yaw, t),
            pitch: self.pitch + (next.pitch - self.pitch) * t,
            roll: self.roll + (next.roll - self.roll) * t,
            steer: self.steer + (next.steer - self.steer) * t,
            wheels: std::array::from_fn(|index| {
                angle_lerp(self.wheels[index], next.wheels[index], t)
            }),
            ground: std::array::from_fn(|index| {
                self.ground[index] + (next.ground[index] - self.ground[index]) * t
            }),
        }
    }
}

/// Each car's pose at the previous tick.
pub struct Poses {
    previous: Vec<Pose>,
}

impl Poses {
    pub fn new(race: &Race) -> Self {
        Self {
            previous: race.entrants.iter().map(|e| Pose::of(&e.car)).collect(),
        }
    }

    /// Call before stepping the race, and every tick while it is paused.
    pub fn record(&mut self, race: &Race) {
        self.previous.clear();
        self.previous
            .extend(race.entrants.iter().map(|e| Pose::of(&e.car)));
    }

    /// Pose of `index` a fraction `t` of a tick after the previous one.
    pub fn at(&self, race: &Race, index: usize, t: f64) -> Pose {
        let current = Pose::of(&race.entrants[index].car);
        match self.previous.get(index) {
            // Recovered cars must not sweep across the track.
            Some(previous) if previous.origin.distance(current.origin) < TELEPORT => {
                let mut pose = previous.lerp(&current, t.clamp(0., 1.));
                pose.resample_ground(&race.track.collision, &race.entrants[index].car.spec.wheels);
                pose
            }
            _ => current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendered_contacts_follow_a_step_instead_of_blending_road_heights() {
        let mut bytes = b"F1CL".to_vec();
        for value in [1_u32, 4, 0] {
            bytes.extend(value.to_le_bytes());
        }
        for (z0, z1, height) in [(-4_f32, 0_f32, 0_f32), (0., 4., 0.2)] {
            let a = [-5., height, z0];
            let b = [5., height, z0];
            let c = [-5., height, z1];
            let d = [5., height, z1];
            for vertex in [a, c, b, b, c, d] {
                for value in vertex {
                    bytes.extend(value.to_le_bytes());
                }
            }
        }
        bytes.extend([0_u8; 4]);
        let collision = racecraft_sim::track::CollisionWorld::from_bytes(&bytes).unwrap();
        let session = crate::tests::session();
        let car = &session.race.entrants[0].car;
        let mut before = Pose::of(car);
        before.origin = DVec3::new(0., 0., -1.8);
        before.yaw = 0.;
        before.pitch = 0.;
        before.roll = 0.;
        before.ground = [0.; 4];
        let mut after = before;
        after.origin.z = -1.6;
        after.ground = [0.2, 0.2, 0., 0.];
        for (fraction, expected) in [(0.25, 0.), (0.75, 0.2)] {
            let mut rendered = before.lerp(&after, fraction);
            assert!((rendered.ground[0] - expected).abs() > 0.01);
            rendered.resample_ground(&collision, &car.spec.wheels);
            for height in &rendered.ground[..2] {
                assert!((*height - expected).abs() < 1e-6);
            }
            assert_eq!(&rendered.ground[2..], &[0., 0.]);
        }
    }

    #[test]
    fn yaw_blends_across_the_wrap_and_teleports_snap() {
        let mut session = crate::tests::session();
        let race = &mut session.race;
        race.entrants[0].car.yaw = PI - 0.01;
        let mut poses = Poses::new(race);
        race.entrants[0].car.yaw = -PI + 0.01;
        let half = poses.at(race, 0, 0.5);
        assert!((half.yaw.rem_euclid(TAU) - PI).abs() < 1e-9);
        poses.record(race);
        race.entrants[0].car.position.x += 40.;
        assert_eq!(poses.at(race, 0, 0.), Pose::of(&race.entrants[0].car));
    }
}
