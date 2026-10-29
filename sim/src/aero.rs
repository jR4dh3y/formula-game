//! Car-to-car aerodynamics: slipstream on straights, dirty air in corners.
//!
//! A leading car leaves a wake that widens behind it. Inside the wake a
//! follower sees less drag (slipstream) and, at speed and close range, less
//! downforce (dirty air). Both fade with distance and lateral offset, so moving
//! partly out of line trades tow for clean air.

use crate::car::{AeroInfluence, Car};
use glam::DVec2;

/// Wake length that still gives a tow, metres.
pub const SLIPSTREAM_RANGE: f64 = 75.0;
/// Range where dirty air costs downforce, metres.
pub const DIRTY_AIR_RANGE: f64 = 40.0;
/// Peak drag reduction right behind another car.
pub const MAX_SLIPSTREAM: f64 = 0.24;
/// Peak front downforce loss right behind another car.
pub const MAX_DIRTY_AIR: f64 = 0.24;

/// Computes every car's influence from every other car.
pub fn influences(cars: &[Car]) -> Vec<AeroInfluence> {
    cars.iter()
        .enumerate()
        .map(|(index, follower)| {
            cars.iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, leader)| influence(follower, leader))
                .fold(AeroInfluence::default(), |total, next| AeroInfluence {
                    drag_scale: total.drag_scale.min(next.drag_scale),
                    front_downforce_scale: total
                        .front_downforce_scale
                        .min(next.front_downforce_scale),
                    rear_downforce_scale: total.rear_downforce_scale.min(next.rear_downforce_scale),
                })
        })
        .collect()
}

/// What `leader`'s wake does to `follower`.
pub fn influence(follower: &Car, leader: &Car) -> AeroInfluence {
    let leader_speed = leader.speed();
    if leader_speed < 12.0 {
        return AeroInfluence::default();
    }
    let wake_axis = leader.velocity / leader_speed;
    let offset = DVec2::new(
        follower.position.x - leader.position.x,
        follower.position.z - leader.position.z,
    );
    let behind = -offset.dot(wake_axis);
    if behind < 3.0 || behind > SLIPSTREAM_RANGE {
        return AeroInfluence::default();
    }
    let lateral = offset.perp_dot(wake_axis).abs();
    let half_width = 1.1 + behind * 0.03;
    let in_wake = (1.0 - (lateral / half_width).powi(2)).max(0.0);
    if in_wake == 0.0 {
        return AeroInfluence::default();
    }
    let tow = MAX_SLIPSTREAM * (1.0 - behind / SLIPSTREAM_RANGE).powf(1.2) * in_wake;
    // Dirty air matters for aerodynamic corners, not hairpins.
    let speed_factor = smoothstep(22.0, 55.0, follower.speed());
    let dirty = if behind < DIRTY_AIR_RANGE {
        MAX_DIRTY_AIR * (1.0 - behind / DIRTY_AIR_RANGE).powf(1.4) * in_wake * speed_factor
    } else {
        0.0
    };
    AeroInfluence {
        drag_scale: 1.0 - tow,
        front_downforce_scale: 1.0 - dirty,
        rear_downforce_scale: 1.0 - dirty * 0.5,
    }
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::spec::{test_wheels, CarSpec, Setup};
    use crate::car::Assists;
    use crate::track::oval_track;
    use glam::DVec3;
    use std::sync::Arc;

    fn pair(gap: f64, lateral: f64) -> (Car, Car) {
        let track = oval_track();
        let spec = Arc::new(CarSpec::rb22(test_wheels(), 5.4, 1.9));
        let make = |position: DVec3| {
            let mut car = Car::new(
                spec.clone(),
                Setup::default(),
                Assists::default(),
                &track,
                position,
                0.0,
            );
            car.velocity = DVec2::new(0.0, 70.0);
            car
        };
        let leader = make(DVec3::new(0.0, 0.0, 0.0));
        let follower = make(DVec3::new(lateral, 0.0, -gap));
        (follower, leader)
    }

    #[test]
    fn close_and_aligned_gives_the_strongest_tow_and_dirty_air() {
        let (follower, leader) = pair(10.0, 0.0);
        let close = influence(&follower, &leader);
        let (follower, leader) = pair(50.0, 0.0);
        let far = influence(&follower, &leader);
        assert!(close.slipstream() > far.slipstream());
        assert!(close.dirty_air() > 0.05);
        assert_eq!(far.dirty_air(), 0.0);
        assert!(far.slipstream() > 0.0);
    }

    #[test]
    fn stepping_out_of_the_wake_recovers_clean_air() {
        let (follower, leader) = pair(15.0, 0.0);
        let aligned = influence(&follower, &leader);
        let (follower, leader) = pair(15.0, 1.2);
        let offset = influence(&follower, &leader);
        let (follower, leader) = pair(15.0, 4.0);
        let clear = influence(&follower, &leader);
        assert!(offset.dirty_air() < aligned.dirty_air());
        assert_eq!(clear, AeroInfluence::default());
    }

    #[test]
    fn leader_is_unaffected_by_follower() {
        let (follower, leader) = pair(10.0, 0.0);
        assert_eq!(influence(&leader, &follower), AeroInfluence::default());
    }
}
