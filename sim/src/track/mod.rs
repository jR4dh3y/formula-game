//! Circuit geometry: centerline stations, drivable surfaces, walls and the racing line.

pub mod collision;
pub mod racing_line;

pub use collision::{CollisionWorld, GroundHit, Surface, Wall};
pub use racing_line::RacingLine;

use glam::{DVec2, DVec3};
use serde::Deserialize;
use std::path::Path;

/// Spacing of resampled centerline stations in metres.
pub const STATION_SPACING: f64 = 2.0;

#[derive(Deserialize)]
struct TrackFile {
    name: String,
    #[serde(default)]
    source: String,
    centerline: Vec<[f64; 3]>,
    half_width: Vec<f64>,
    #[serde(default)]
    corners: Vec<CornerFile>,
    #[serde(default = "default_sun")]
    sun_direction: [f64; 3],
}

#[derive(Deserialize)]
struct CornerFile {
    name: String,
    distance: f64,
}

fn default_sun() -> [f64; 3] {
    [-0.35, -0.78, -0.52]
}

#[derive(Clone, Copy, Debug)]
pub struct Station {
    /// Distance from the start line along the centerline.
    pub s: f64,
    pub position: DVec3,
    /// Unit direction of travel in the ground plane (x, z).
    pub tangent: DVec2,
    /// Half of the asphalt width.
    pub half_width: f64,
}

impl Station {
    /// Unit vector pointing to the driver's left in the ground plane.
    pub fn left(&self) -> DVec2 {
        DVec2::new(self.tangent.y, -self.tangent.x)
    }

    pub fn point_at(&self, lateral: f64) -> DVec3 {
        let left = self.left() * lateral;
        DVec3::new(
            self.position.x + left.x,
            self.position.y,
            self.position.z + left.y,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Corner {
    pub name: String,
    pub distance: f64,
}

/// Where a point lies relative to the centerline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPosition {
    pub station: usize,
    /// Distance along the lap in `[0, length)`.
    pub s: f64,
    /// Signed offset from the centerline, positive to the left.
    pub lateral: f64,
}

pub struct Track {
    pub name: String,
    pub source: String,
    pub length: f64,
    pub stations: Vec<Station>,
    pub corners: Vec<Corner>,
    pub collision: CollisionWorld,
    pub racing_line: RacingLine,
    pub sun_direction: DVec3,
}

impl Track {
    /// Loads `track.json` and `collision.bin` from a converted track directory.
    pub fn load(directory: impl AsRef<Path>) -> Result<Self, String> {
        let directory = directory.as_ref();
        let read = |name: &str| {
            std::fs::read(directory.join(name))
                .map_err(|error| format!("{}: {error}", directory.join(name).display()))
        };
        let file: TrackFile = serde_json::from_slice(&read("track.json")?)
            .map_err(|error| format!("track.json: {error}"))?;
        let collision = CollisionWorld::from_bytes(&read("collision.bin")?)?;
        Self::from_parts(file, collision)
    }

    fn from_parts(file: TrackFile, collision: CollisionWorld) -> Result<Self, String> {
        if file.centerline.len() < 8 || file.centerline.len() != file.half_width.len() {
            return Err("track centerline and widths are inconsistent".to_string());
        }
        let points: Vec<DVec3> = file.centerline.iter().map(|&p| DVec3::from(p)).collect();
        let stations = resample(&points, &file.half_width);
        let length = stations.len() as f64 * STATION_SPACING;
        let racing_line = RacingLine::compute(&stations);
        let sun = DVec3::from(file.sun_direction).normalize_or(DVec3::NEG_Y);
        Ok(Self {
            name: file.name,
            source: file.source,
            length,
            stations,
            corners: file
                .corners
                .into_iter()
                .map(|corner| Corner {
                    name: corner.name,
                    distance: corner.distance,
                })
                .collect(),
            collision,
            racing_line,
            sun_direction: sun,
        })
    }

    pub fn station(&self, index: usize) -> &Station {
        &self.stations[index % self.stations.len()]
    }

    /// Station index at a lap distance (wraps).
    pub fn station_at(&self, s: f64) -> usize {
        (s.rem_euclid(self.length) / STATION_SPACING) as usize % self.stations.len()
    }

    /// Wraps a distance into `[0, length)`.
    pub fn wrap(&self, s: f64) -> f64 {
        s.rem_euclid(self.length)
    }

    /// Signed shortest distance from `from` to `to` along the lap.
    pub fn delta(&self, from: f64, to: f64) -> f64 {
        let delta = (to - from).rem_euclid(self.length);
        if delta > self.length * 0.5 {
            delta - self.length
        } else {
            delta
        }
    }

    /// Interpolated centerline frame at a lap distance.
    pub fn frame_at(&self, s: f64) -> Station {
        let s = self.wrap(s);
        let index = (s / STATION_SPACING) as usize % self.stations.len();
        let next = (index + 1) % self.stations.len();
        let t = (s - index as f64 * STATION_SPACING) / STATION_SPACING;
        let (a, b) = (&self.stations[index], &self.stations[next]);
        Station {
            s,
            position: a.position.lerp(b.position, t),
            tangent: a.tangent.lerp(b.tangent, t).normalize_or(a.tangent),
            half_width: a.half_width + (b.half_width - a.half_width) * t,
        }
    }

    /// Projects a world position onto the centerline, searching near `hint`.
    pub fn project(&self, position: DVec3, hint: Option<usize>) -> TrackPosition {
        let point = DVec2::new(position.x, position.z);
        let count = self.stations.len();
        let best = match hint {
            Some(hint) => self.nearest_in_window(point, hint, 40),
            None => (0..count)
                .min_by(|&a, &b| {
                    let da = DVec2::new(self.stations[a].position.x, self.stations[a].position.z)
                        .distance_squared(point);
                    let db = DVec2::new(self.stations[b].position.x, self.stations[b].position.z)
                        .distance_squared(point);
                    da.total_cmp(&db)
                })
                .expect("tracks have stations"),
        };
        // On the segments either side of `best`, find where the interpolated
        // normal passes through the point. Measuring along a single station's
        // tangent instead makes `s` jump backwards at each station when the
        // car is well off the centreline in a curve.
        let mut chosen: Option<(f64, f64)> = None;
        for index in [(best + count - 1) % count, best] {
            let (a, b) = (&self.stations[index], &self.stations[(index + 1) % count]);
            let start = DVec2::new(a.position.x, a.position.z);
            let edge = DVec2::new(b.position.x, b.position.z) - start;
            let turn = b.tangent - a.tangent;
            let offset = point - start;
            // (offset - t·edge) · (tangent_a + t·turn) = 0
            let (c0, c1, c2) = (
                offset.dot(a.tangent),
                offset.dot(turn) - edge.dot(a.tangent),
                -edge.dot(turn),
            );
            let roots = if c2.abs() < 1e-12 {
                [-c0 / c1, f64::NAN]
            } else {
                let discriminant = c1 * c1 - 4.0 * c2 * c0;
                if discriminant < 0.0 {
                    continue;
                }
                let root = discriminant.sqrt();
                [(-c1 + root) / (2.0 * c2), (-c1 - root) / (2.0 * c2)]
            };
            for t in roots {
                if !(-1e-9..=1.0 + 1e-9).contains(&t) {
                    continue;
                }
                let distance = (offset - edge * t).length_squared();
                if chosen.is_none_or(|(_, best)| distance < best) {
                    chosen = Some((
                        (index as f64 + t.clamp(0.0, 1.0)) * STATION_SPACING,
                        distance,
                    ));
                }
            }
        }
        let s = chosen.map(|(s, _)| s).unwrap_or_else(|| {
            let station = &self.stations[best];
            let origin = DVec2::new(station.position.x, station.position.z);
            best as f64 * STATION_SPACING + (point - origin).dot(station.tangent)
        });
        let frame = self.frame_at(s);
        let lateral = (point - DVec2::new(frame.position.x, frame.position.z)).dot(frame.left());
        TrackPosition {
            station: self.station_at(frame.s),
            s: frame.s,
            lateral,
        }
    }

    fn nearest_in_window(&self, point: DVec2, hint: usize, window: usize) -> usize {
        let count = self.stations.len();
        (0..=window * 2)
            .map(|offset| (hint + count + offset - window) % count)
            .min_by(|&a, &b| {
                let da = DVec2::new(self.stations[a].position.x, self.stations[a].position.z)
                    .distance_squared(point);
                let db = DVec2::new(self.stations[b].position.x, self.stations[b].position.z)
                    .distance_squared(point);
                da.total_cmp(&db)
            })
            .expect("window is non-empty")
    }

    /// The named corner a driver at `s` is in or approaching (within 150 m).
    pub fn corner_near(&self, s: f64) -> Option<&Corner> {
        self.corners
            .iter()
            .filter(|corner| {
                let ahead = self.delta(s, corner.distance);
                (-60.0..150.0).contains(&ahead)
            })
            .min_by(|a, b| {
                self.delta(s, a.distance)
                    .abs()
                    .total_cmp(&self.delta(s, b.distance).abs())
            })
    }

    /// Starting grid slot `index` (0 = pole): staggered two-by-two behind the line.
    pub fn grid_slot(&self, index: usize) -> (DVec3, f64) {
        let s = -12.0 - index as f64 * 8.0;
        let frame = self.frame_at(s);
        let side = if index.is_multiple_of(2) { 1.0 } else { -1.0 };
        let lateral = side * (frame.half_width * 0.45).min(3.0);
        let position = frame.point_at(lateral);
        let ground = self
            .collision
            .ground(position + DVec3::Y * 2.0)
            .map_or(position.y, |hit| hit.height);
        let yaw = frame.tangent.x.atan2(frame.tangent.y);
        (DVec3::new(position.x, ground, position.z), yaw)
    }
}

/// Resamples a closed polyline into stations every `STATION_SPACING` metres.
fn resample(points: &[DVec3], half_widths: &[f64]) -> Vec<Station> {
    let count = points.len();
    let mut cumulative = Vec::with_capacity(count + 1);
    let mut total = 0.0;
    cumulative.push(0.0);
    for index in 0..count {
        let next = points[(index + 1) % count];
        total += DVec2::new(next.x - points[index].x, next.z - points[index].z).length();
        cumulative.push(total);
    }
    let station_count = (total / STATION_SPACING).round() as usize;
    let scale = total / (station_count as f64 * STATION_SPACING);
    let mut stations = Vec::with_capacity(station_count);
    let mut segment = 0;
    for station in 0..station_count {
        let target = station as f64 * STATION_SPACING * scale;
        while cumulative[segment + 1] < target {
            segment += 1;
        }
        let span = (cumulative[segment + 1] - cumulative[segment]).max(1e-9);
        let t = (target - cumulative[segment]) / span;
        let a = points[segment];
        let b = points[(segment + 1) % count];
        stations.push(Station {
            s: station as f64 * STATION_SPACING,
            position: a.lerp(b, t),
            tangent: DVec2::ZERO,
            half_width: half_widths[segment]
                + (half_widths[(segment + 1) % count] - half_widths[segment]) * t,
        });
    }
    // Central-difference tangents over a few metres smooth out polyline kinks.
    let n = stations.len();
    for index in 0..n {
        let ahead = stations[(index + 2) % n].position;
        let behind = stations[(index + n - 2) % n].position;
        stations[index].tangent = DVec2::new(ahead.x - behind.x, ahead.z - behind.z).normalize();
    }
    stations
}

#[cfg(test)]
pub(crate) fn oval_track() -> Track {
    Track::with_walls_for_test(Vec::new())
}

#[cfg(test)]
impl Track {
    pub(crate) fn with_walls_for_test(walls: Vec<Wall>) -> Track {
        oval_with_walls(walls)
    }
}

#[cfg(test)]
fn oval_with_walls(walls: Vec<Wall>) -> Track {
    // 200 m straights joined by 60 m radius semicircles, 6 m half width.
    stadium(200.0, 60.0, walls)
}

/// Two `straight`-metre straights joined by semicircles of `radius`.
#[cfg(test)]
pub(crate) fn stadium(straight: f64, radius: f64, walls: Vec<Wall>) -> Track {
    let mut centerline = Vec::new();
    let half = straight / 2.0;
    let points = (straight / 2.0) as usize;
    for i in 0..points {
        centerline.push([i as f64 * 2.0 - half, 0.0, -radius]);
    }
    for i in 0..95 {
        let angle = -std::f64::consts::FRAC_PI_2 + i as f64 / 95.0 * std::f64::consts::PI;
        centerline.push([half + radius * angle.cos(), 0.0, radius * angle.sin()]);
    }
    for i in 0..points {
        centerline.push([half - i as f64 * 2.0, 0.0, radius]);
    }
    for i in 0..95 {
        let angle = std::f64::consts::FRAC_PI_2 + i as f64 / 95.0 * std::f64::consts::PI;
        centerline.push([-half + radius * angle.cos(), 0.0, radius * angle.sin()]);
    }
    let half_width = vec![6.0; centerline.len()];
    let file = TrackFile {
        name: "Test Oval".to_string(),
        source: String::new(),
        centerline,
        half_width,
        corners: vec![CornerFile {
            name: "East".to_string(),
            distance: 200.0,
        }],
        sun_direction: default_sun(),
    };
    Track::from_parts(file, collision::flat_world(400.0, Surface::Asphalt, walls)).expect("oval")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_recovers_distance_and_lateral_offset() {
        let track = oval_track();
        let frame = track.frame_at(57.0);
        let point = frame.point_at(2.5);
        let projected = track.project(point, None);
        assert!((projected.s - 57.0).abs() < 0.05, "{projected:?}");
        assert!((projected.lateral - 2.5).abs() < 0.05, "{projected:?}");
        let hinted = track.project(point, Some(projected.station + 3));
        assert!((hinted.s - projected.s).abs() < 1e-9);
    }

    #[test]
    fn delta_wraps_across_the_line() {
        let track = oval_track();
        assert!((track.delta(track.length - 5.0, 5.0) - 10.0).abs() < 1e-9);
        assert!((track.delta(5.0, track.length - 5.0) + 10.0).abs() < 1e-9);
    }

    #[test]
    fn grid_slots_sit_behind_the_line_facing_forward() {
        let track = oval_track();
        let (pole, yaw) = track.grid_slot(0);
        let (second, _) = track.grid_slot(1);
        let pole_s = track.project(pole, None).s;
        assert!(track.delta(pole_s, 0.0) > 0.0);
        assert!(track.project(second, None).lateral < 0.0);
        let tangent = track.frame_at(pole_s).tangent;
        assert!((DVec2::new(yaw.sin(), yaw.cos()).dot(tangent) - 1.0).abs() < 1e-3);
    }
}
