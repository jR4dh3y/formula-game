//! Drag reduction system: activation zones on a circuit's long straights and
//! the one-second rule at each zone's detection point.
//!
//! Circuit data carries no official zones, so they are found from the
//! centerline: the longest runs whose heading barely changes.

use crate::track::{Track, STATION_SPACING};

/// Eligible when at most this many seconds behind the car ahead at detection.
pub const GAP: f64 = 1.0;
/// DRS is enabled once a car has completed this many laps.
pub const ENABLED_AFTER_LAPS: u32 = 1;
/// Seconds for the flap to open or close fully.
pub const ACTUATION: f64 = 0.25;

/// Heading change below this (rad per metre, about a 650 m radius) is straight.
const STRAIGHT_RATE: f64 = 0.0015;
/// Heading is compared this far either side of each station.
const WINDOW: f64 = 20.0;
const MIN_STRAIGHT: f64 = 400.0;
/// Flat-out kinks this short and gentle do not split a straight (Spa's Kemmel).
const MAX_KINK: f64 = 80.0;
const MAX_KINK_TURN: f64 = 0.26;
/// Activation starts after the corner exit and ends before the braking zone.
const ENTRY: f64 = 60.0;
const EXIT: f64 = 100.0;
/// Detection sits this far before activation, usually at the previous corner.
const DETECTION: f64 = 150.0;
const MAX_ZONES: usize = 2;
/// A zone must end at a braking corner: a racing-line radius of 60 m or less
/// within this distance. Flat-out bends (Eau Rouge, Blanchimont) do not count.
const BRAKING_CURVATURE: f64 = 1.0 / 60.0;
const BRAKING_SEARCH: f64 = 180.0;

/// Lap distances along the centerline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub detection: f64,
    pub start: f64,
    pub end: f64,
}

impl Zone {
    pub fn contains(&self, track: &Track, s: f64) -> bool {
        (s - self.start).rem_euclid(track.length)
            <= (self.end - self.start).rem_euclid(track.length)
    }

    pub fn length(&self, track: &Track) -> f64 {
        (self.end - self.start).rem_euclid(track.length)
    }
}

/// The circuit's longest straights as DRS zones, in lap order.
pub fn zones(track: &Track) -> Vec<Zone> {
    let count = track.stations.len();
    let reach = (WINDOW / STATION_SPACING).ceil() as usize;
    if count < reach * 4 {
        return Vec::new();
    }
    let straight: Vec<bool> = (0..count)
        .map(|index| {
            let before = track.station(index + count - reach).tangent;
            let after = track.station(index + reach).tangent;
            let turn = before.angle_to(after).abs();
            turn / (2.0 * reach as f64 * STATION_SPACING) < STRAIGHT_RATE
        })
        .collect();
    let Some(first_bend) = straight.iter().position(|&s| !s) else {
        return Vec::new();
    };
    // Walk once around the lap from a bend so no run is split at the start line.
    let mut runs = Vec::new();
    let mut run_start = None;
    for offset in 1..=count {
        let index = (first_bend + offset) % count;
        match (straight[index], run_start) {
            (true, None) => run_start = Some(offset),
            (false, Some(start)) => {
                runs.push((start, offset - start));
                run_start = None;
            }
            _ => {}
        }
    }
    // Merge runs split by a kink, including across the walk's starting bend.
    let heading = |offset: usize| track.station(first_bend + offset).tangent;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, length) in runs {
        if let Some(last) = merged.last_mut() {
            let gap = start - (last.0 + last.1);
            let turn = heading(last.0 + last.1 - 1).angle_to(heading(start)).abs();
            if gap as f64 * STATION_SPACING <= MAX_KINK && turn <= MAX_KINK_TURN {
                last.1 = start + length - last.0;
                continue;
            }
        }
        merged.push((start, length));
    }
    if merged.len() > 1 {
        let (first, last) = (merged[0], merged[merged.len() - 1]);
        let gap = first.0 + count - (last.0 + last.1);
        let turn = heading(last.0 + last.1 - 1)
            .angle_to(heading(first.0))
            .abs();
        if gap as f64 * STATION_SPACING <= MAX_KINK && turn <= MAX_KINK_TURN {
            merged[0] = (last.0, first.0 + count + first.1 - last.0);
            merged.pop();
        }
    }
    let mut runs: Vec<(f64, f64)> = merged
        .into_iter()
        .map(|(start, stations)| {
            (
                track.wrap((first_bend + start) as f64 * STATION_SPACING),
                stations as f64 * STATION_SPACING,
            )
        })
        .filter(|&(start, length)| length >= MIN_STRAIGHT && ends_in_braking(track, start + length))
        .collect();
    runs.sort_by(|a, b| b.1.total_cmp(&a.1));
    runs.truncate(MAX_ZONES);
    let mut zones: Vec<Zone> = runs
        .into_iter()
        .map(|(start, length)| Zone {
            detection: track.wrap(start + ENTRY - DETECTION),
            start: track.wrap(start + ENTRY),
            end: track.wrap(start + length - EXIT),
        })
        .collect();
    zones.sort_by(|a, b| a.start.total_cmp(&b.start));
    zones
}

fn ends_in_braking(track: &Track, end: f64) -> bool {
    let first = track.station_at(end);
    let curvature = &track.racing_line.curvature;
    (0..(BRAKING_SEARCH / STATION_SPACING) as usize)
        .any(|k| curvature[(first + k) % curvature.len()].abs() >= BRAKING_CURVATURE)
}

/// Per-car eligibility, earned at detection and spent at the end of a zone.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    eligible: Vec<bool>,
    last_s: Option<f64>,
}

impl Tracker {
    /// Moves the car to lap distance `s`. `qualifies` is evaluated only when a
    /// detection point is crossed. Returns whether DRS may be open now.
    pub fn update(
        &mut self,
        track: &Track,
        zones: &[Zone],
        s: f64,
        qualifies: impl Fn() -> bool,
    ) -> bool {
        self.eligible.resize(zones.len(), false);
        if let Some(last) = self.last_s {
            let moved = track.delta(last, s);
            // Recoveries teleport cars; only forward driving crosses a line.
            let crossed = |mark: f64| {
                let ahead = (mark - last).rem_euclid(track.length);
                moved > 0.0 && moved < 50.0 && ahead > 0.0 && ahead <= moved
            };
            for (zone, eligible) in zones.iter().zip(&mut self.eligible) {
                if crossed(zone.detection) {
                    *eligible = qualifies();
                }
                if crossed(zone.end) {
                    *eligible = false;
                }
            }
        }
        self.last_s = Some(s);
        zones
            .iter()
            .zip(&self.eligible)
            .any(|(zone, &eligible)| eligible && zone.contains(track, s))
    }

    /// Withdraws any eligibility already earned, e.g. after a recovery.
    pub fn reset(&mut self) {
        self.eligible.fill(false);
        self.last_s = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{oval_track, stadium};

    #[test]
    fn zones_cover_long_straights_but_not_short_ones() {
        assert!(zones(&oval_track()).is_empty());
        // Long straights into fast sweepers need no braking, so no DRS either.
        assert!(zones(&stadium(1000.0, 150.0, Vec::new())).is_empty());
        let track = stadium(1000.0, 30.0, Vec::new());
        let found = zones(&track);
        assert_eq!(found.len(), 2, "{found:?}");
        for zone in &found {
            let length = zone.length(&track);
            assert!((700.0..900.0).contains(&length), "{zone:?} {length}");
            // Both ends sit on a straight, away from the bends.
            for s in [zone.start, zone.end] {
                let tangent = track.frame_at(s).tangent;
                assert!(tangent.y.abs() < 0.01, "{s}: {tangent}");
            }
            assert!((track.delta(zone.detection, zone.start) - DETECTION).abs() < 1e-6);
        }
    }

    #[test]
    fn eligibility_is_earned_at_detection_and_ends_with_the_zone() {
        let track = stadium(1000.0, 30.0, Vec::new());
        let zone = zones(&track)[0];
        let zones = [zone];
        let drive = |tracker: &mut Tracker, from: f64, to: f64, qualifies: bool| {
            let mut s = from;
            let mut open_somewhere = false;
            while track.delta(s, to) > 0.0 {
                s = track.wrap(s + 5.0);
                open_somewhere |= tracker.update(&track, &zones, s, || qualifies);
            }
            open_somewhere
        };
        let before = zone.detection - 20.0;
        let mut tracker = Tracker::default();
        assert!(!drive(&mut tracker, before, zone.end + 30.0, false));
        let mut tracker = Tracker::default();
        tracker.update(&track, &zones, track.wrap(before), || true);
        assert!(!tracker.update(&track, &zones, track.wrap(zone.detection + 5.0), || true));
        assert!(tracker.update(&track, &zones, track.wrap(zone.start + 5.0), || false));
        assert!(drive(&mut tracker, zone.start + 5.0, zone.end - 5.0, false));
        // Past the end the flap must close and stay closed until earned again.
        assert!(!tracker.update(&track, &zones, track.wrap(zone.end + 3.0), || true));
        assert!(!tracker.update(&track, &zones, track.wrap(zone.start + 10.0), || true));
        // Teleports are not driving and do not cross detection.
        let mut tracker = Tracker::default();
        tracker.update(&track, &zones, track.wrap(before), || true);
        tracker.update(&track, &zones, track.wrap(zone.start + 5.0), || true);
        assert!(!tracker.update(&track, &zones, track.wrap(zone.start + 6.0), || true));
    }
}
