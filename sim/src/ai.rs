//! Computer drivers that race rather than follow a line.

use crate::car::{Car, CarInput};
use crate::rng::Rng;
use crate::track::racing_line::{PerformanceEnvelope, RacingLine};
use crate::track::{Track, STATION_SPACING};
use glam::DVec2;

const PLAN_INTERVAL: f64 = 0.25;
/// Maximum lateral distance the tactical offset can move in one second.
const LANE_CHANGE_RATE: f64 = 2.4;

/// Personality of a computer driver.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DriverProfile {
    /// Grip used in braking corners, ~0.9..1.0. Power-limited bends stay flat.
    pub pace: f64,
    /// Willingness to attack and to defend, 0..1.
    pub aggression: f64,
    /// 1 = never errs; lower values make occasional small mistakes.
    pub consistency: f64,
}

/// A corner as the AI sees it: where to brake, where the apex is and which side is inside.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CornerPlan {
    brake_s: f64,
    apex_s: f64,
    /// +1 for a left-hander (inside on the left), -1 for a right-hander.
    inside: f64,
    apex_speed: f64,
    /// A real braking zone where passes are made, not a flat or lifted kink.
    passing: bool,
}

/// Approach speed over apex speed that makes a corner a passing place.
const PASSING_SPEED_DROP: f64 = 1.4;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Intent {
    /// Drive the racing line.
    Line,
    /// Line up a pass on the given side (+1 left) for the corner at `apex_s`.
    Attack { side: f64, apex_s: f64 },
    /// Cover the inside of the corner at `apex_s` (one move, held).
    Defend { side: f64, apex_s: f64 },
}

/// What a driver knows about another car this tick.
#[derive(Clone, Copy, Debug)]
pub struct Rival {
    /// Track distance from us to them, positive when they are ahead.
    pub gap: f64,
    pub lateral: f64,
    pub speed: f64,
    pub position: DVec2,
}

#[derive(Clone, Debug)]
pub struct AiDriver {
    pub profile: DriverProfile,
    speed_profile: Vec<f64>,
    passing_speeds: Option<[Vec<f64>; 2]>,
    corners: Vec<CornerPlan>,
    pub intent: Intent,
    rng: Rng,
    /// Late-braking error for the next corner, metres.
    mistake: f64,
    mistake_corner: Option<f64>,
    plan_timer: f64,
    stuck_timer: f64,
    /// Smoothed tactical offset from the racing line, metres.
    line_offset: f64,
    /// One tactical move for this braking zone, even if a threat disappears.
    moved_for_corner: Option<f64>,
    /// Seconds spent stuck behind a car in our lane: held back by it, or
    /// gaining on it in its tow. A patient follower still pulls out to pass.
    held_up: f64,
    /// The attacker has left the tow for the passing lane and stays there
    /// until the move is completed or abandoned.
    pulled_out: bool,
    /// Tyre grip the current speed profile was planned for.
    planned_grip: f64,
    /// Set while the race holds the car on the grid.
    on_grid: bool,
    /// The grid lane kept from the start through the first corner.
    launch: Option<Launch>,
    /// The plan this driver used before the current one. Network rollback can
    /// move a driver back and forth across a replanning threshold; swapping to
    /// this copy avoids rebuilding the whole-lap profile each time.
    previous_plan: Option<Box<Plan>>,
}

#[derive(Clone, Debug)]
struct Plan {
    profile: DriverProfile,
    grip: f64,
    speed_profile: Vec<f64>,
    passing_speeds: Option<[Vec<f64>; 2]>,
    corners: Vec<CornerPlan>,
}

/// A standing start keeps each car in its grid lane, so the field runs two
/// wide into the first corner instead of collapsing onto one racing line.
#[derive(Clone, Debug)]
struct Launch {
    state: LaunchState,
    /// Speeds for the lane's own path, which is tighter than the racing
    /// line through a corner taken from one side of the road. Cached and
    /// rebuilt locally rather than sent over the network.
    speed: Vec<f64>,
    line: RacingLine,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct LaunchState {
    lane: f64,
    from_s: f64,
    release_s: f64,
    profile: DriverProfile,
    grip: f64,
}

/// Metres past the first corner's apex before a car may rejoin the line.
const LAUNCH_RELEASE: f64 = 40.0;
/// A car still boxed in this far past the release point rejoins anyway.
const LAUNCH_HOLD_LIMIT: f64 = 500.0;

impl AiDriver {
    pub fn new(profile: DriverProfile, car: &Car, track: &Track, seed: u64) -> Self {
        let planned_grip = tyre_grip(car);
        let speed_profile = speed_profile(profile, car, &track.racing_line, planned_grip);
        let passing_speeds = passing_speeds(profile, car, track, planned_grip);
        let corners = find_corners(track, &speed_profile);
        let mut rng = Rng::new(seed);
        // Stagger replanning so a full field never plans on the same tick.
        let plan_timer = rng.next_f64() * PLAN_INTERVAL;
        Self {
            profile,
            speed_profile,
            passing_speeds,
            corners,
            intent: Intent::Line,
            rng,
            mistake: 0.0,
            mistake_corner: None,
            plan_timer,
            stuck_timer: 0.0,
            line_offset: 0.0,
            moved_for_corner: None,
            held_up: 0.0,
            pulled_out: false,
            planned_grip,
            on_grid: false,
            launch: None,
            previous_plan: None,
        }
    }

    /// Switch to the lap plan for `profile` and `grip`, reusing the previous plan when it matches.
    fn set_plan(&mut self, profile: DriverProfile, grip: f64, car: &Car, track: &Track) {
        if self.profile == profile && self.planned_grip == grip {
            return;
        }
        let current = Plan {
            profile: self.profile,
            grip: self.planned_grip,
            speed_profile: std::mem::take(&mut self.speed_profile),
            passing_speeds: self.passing_speeds.take(),
            corners: std::mem::take(&mut self.corners),
        };
        let next = match self.previous_plan.take() {
            Some(plan) if plan.profile == profile && plan.grip == grip => *plan,
            _ => {
                let speed_profile = speed_profile(profile, car, &track.racing_line, grip);
                let corners = find_corners(track, &speed_profile);
                let passing_speeds = passing_speeds(profile, car, track, grip);
                Plan {
                    passing_speeds,
                    profile,
                    grip,
                    speed_profile,
                    corners,
                }
            }
        };
        self.profile = next.profile;
        self.planned_grip = next.grip;
        self.speed_profile = next.speed_profile;
        self.passing_speeds = next.passing_speeds;
        self.corners = next.corners;
        self.previous_plan = Some(Box::new(current));
    }

    /// Target speed the driver wants at lap distance `s`.
    pub fn target_speed(&self, track: &Track, s: f64) -> f64 {
        self.speed_profile[track.station_at(s)]
    }

    fn next_corner(&self, track: &Track, s: f64) -> Option<CornerPlan> {
        self.corners
            .iter()
            .copied()
            .filter(|corner| corner.passing && track.delta(s, corner.apex_s) > -10.0)
            .min_by(|a, b| {
                track
                    .delta(s, a.apex_s)
                    .rem_euclid(track.length)
                    .total_cmp(&track.delta(s, b.apex_s).rem_euclid(track.length))
            })
    }

    /// Seconds the car has been nearly stationary while it should be racing.
    pub fn stuck_time(&self) -> f64 {
        self.stuck_timer
    }

    /// Held on the grid: standing still is not being stuck.
    pub fn hold(&mut self) {
        self.stuck_timer = 0.0;
        self.on_grid = true;
    }

    pub fn reset_after_recovery(&mut self) {
        self.stuck_timer = 0.0;
        self.intent = Intent::Line;
        self.launch = None;
        self.moved_for_corner = None;
        self.line_offset = 0.;
        self.held_up = 0.0;
        self.pulled_out = false;
    }

    pub fn drive(&mut self, car: &Car, track: &Track, rivals: &[Rival], dt: f64) -> CarInput {
        let s = car.track_position.s;
        let speed = car.speed();
        let corner = self.next_corner(track, s);
        if std::mem::take(&mut self.on_grid) {
            // Still held: remember the grid lane, its speeds, and where the
            // first corner ends. Replan only if the car has moved lanes.
            let lane = car.track_position.lateral;
            let moved = self
                .launch
                .as_ref()
                .is_none_or(|launch| (launch.state.lane - lane).abs() > 0.2);
            if moved {
                let line = RacingLine::constant_offset(&track.stations, lane);
                self.launch = Some(Launch {
                    state: LaunchState {
                        lane,
                        from_s: s,
                        release_s: corner.map_or(s + 200., |corner| corner.apex_s + LAUNCH_RELEASE),
                        profile: self.profile,
                        grip: self.planned_grip,
                    },
                    speed: speed_profile(self.profile, car, &line, self.planned_grip),
                    line,
                });
            }
        }
        if let Some(launch) = &self.launch {
            let travelled = track.delta(launch.state.from_s, s).rem_euclid(track.length);
            let to_release = track
                .delta(launch.state.from_s, launch.state.release_s)
                .rem_euclid(track.length);
            let line = track.racing_line.offsets[track.station_at(s)];
            let toward = line - car.track_position.lateral;
            // Rejoin only with nobody alongside on the side of the line.
            let boxed_in = rivals.iter().any(|rival| {
                let across = rival.lateral - car.track_position.lateral;
                rival.gap.abs() < car.spec.length * 1.5
                    && across * toward > 0.0
                    && across.abs() < toward.abs() + car.spec.width + 1.0
            });
            let past = travelled > to_release && travelled < track.length * 0.5;
            if past && (!boxed_in || travelled > to_release + LAUNCH_HOLD_LIMIT) {
                self.launch = None;
            }
        }

        self.plan_timer -= dt;
        if self.plan_timer <= 0.0 {
            self.plan_timer += PLAN_INTERVAL;
            self.replan(car, track, rivals, corner);
            if !matches!(self.intent, Intent::Attack { .. }) {
                self.pulled_out = false;
            }
        }
        self.stuck_timer = if speed < 2.0 {
            self.stuck_timer + dt
        } else {
            0.0
        };

        // Lateral target: racing line, or the tactical side, then leave room.
        let lookahead = (6.0 + speed * 0.42).clamp(9.0, 42.0);
        let line_lateral = track.racing_line.lateral_at(s + lookahead);
        let ahead_frame = track.frame_at(s + lookahead);
        let limit = (ahead_frame.half_width - 1.4).max(0.0);
        // Attackers stay in the tow on the optimal line and only take the
        // passing line once they pull out; defenders commit to the inside line.
        let lane = self.tactical_line(track);
        let mut on_lane = false;
        let mut lateral = match self.intent {
            Intent::Line => self
                .launch
                .as_ref()
                .map_or(line_lateral, |launch| launch.state.lane),
            Intent::Attack { side, .. } => {
                // Sit in the tow until close or the braking zone nears, then
                // pull out and use the extra speed to get alongside.
                let to_brake = corner.map_or(f64::INFINITY, |corner| {
                    track.delta(s, corner.brake_s).rem_euclid(track.length)
                });
                let target = rivals
                    .iter()
                    .filter(|rival| rival.gap > 0.0 && rival.gap < 45.0)
                    .min_by(|a, b| a.gap.total_cmp(&b.gap));
                match target {
                    Some(target) => {
                        let closing = speed - target.speed;
                        // Pull out once the tow has brought us close and we are
                        // still gaining, or as the braking zone nears.
                        // A car that has been held up goes now: waiting in
                        // the tow for the leader to give way never passes.
                        self.pulled_out |= to_brake < 60.0 + speed * 1.2
                            || target.gap < car.spec.length * 3.0 && closing > 0.3
                            || target.gap < car.spec.length * 1.5
                            || self.held_up > self.patience()
                                && target.gap < car.spec.length * HELD_UP_PULL_OUT;
                        if self.pulled_out {
                            // Alongside, not out on the kerb: one car width
                            // plus margin beside the car being passed.
                            let beside = target.lateral + side * (car.spec.width + 1.3);
                            // A passing lane is planned for the corner it
                            // attacks; elsewhere its path can be slower than
                            // the straight's racing line. Until the braking
                            // approach, ride alongside the target instead.
                            let approach = to_brake < 60.0 + speed * 1.2;
                            match lane {
                                // Follow the passing line through the corner,
                                // but never closer than alongside the target.
                                Some(lane) if approach => {
                                    on_lane = true;
                                    let path = lane.lateral_at(s + lookahead);
                                    if side > 0.0 {
                                        path.max(beside)
                                    } else {
                                        path.min(beside)
                                    }
                                }
                                _ => beside,
                            }
                        } else {
                            target.lateral
                        }
                    }
                    None => line_lateral,
                }
            }
            Intent::Defend { side, .. } => match lane {
                Some(lane) => {
                    on_lane = true;
                    lane.lateral_at(s + lookahead)
                }
                // Cover the inside down the straight. From the braking point
                // take the corner on the line (the room owed to a car
                // alongside still holds): turning in from the far inside
                // edge is a tighter arc than the plan brakes for.
                None => {
                    let cover = side * (limit - 1.2);
                    let braking = corner.is_some_and(|corner| {
                        track.delta(s, corner.brake_s).rem_euclid(track.length) > track.length * 0.5
                    });
                    if braking {
                        line_lateral
                    } else if side > 0.0 {
                        cover.max(line_lateral)
                    } else {
                        cover.min(line_lateral)
                    }
                }
            },
        };
        let mut lane_min = -limit;
        let mut lane_max = limit;
        // Slowest car level with or ahead of us that we must leave room for.
        let mut beside_speed = f64::INFINITY;
        // Already closer than the room owed to a car alongside: steer back
        // out sharply instead of easing over through the usual lookahead.
        let mut too_close = false;
        // Speed to drop to behind a car alongside that we are closing in on
        // sideways: the car further back backs out instead of leaning on it.
        let mut yield_speed = f64::INFINITY;
        // Our own sideways drift: through a direction change the line
        // carries the car across the road faster than the steering undoes.
        let drift = car.velocity.dot(track.frame_at(s).left()) * LATERAL_PREVIEW;
        let attacking = matches!(self.intent, Intent::Attack { .. });
        for rival in rivals {
            let separation = rival.lateral - car.track_position.lateral;
            // Nose-to-tail traffic must brake, not have both drivers swerve
            // toward the same arbitrary side when their lateral positions tie.
            if separation.abs() < car.spec.width * 0.5 {
                continue;
            }
            // Centres one car length apart is nose to tail, not alongside.
            let reach = car.spec.length + 0.5;
            let overlapping = rival.gap.abs() < reach;
            // Speed at which the gap shrinks toward an overlap, either way.
            let approach = if rival.gap > 0.0 {
                speed - rival.speed
            } else {
                rival.speed - speed
            };
            // A car in another lane that will be alongside within the
            // horizon gets its room now: the line must not swing across into
            // it while the steering catches up. A car we have just passed
            // keeps its lane until we are clear ahead, so the passer does
            // not cut back across its nose. A car behind counts as soon as
            // it is clearly offset, so a leader drifting back to its line
            // does not chop a car pulling out of its tow; a car ahead needs
            // a clearer offset, or a follower in its tow would weave.
            let in_other_lane = rival.gap < 0.0 || separation.abs() > car.spec.width * 0.75;
            let arriving =
                approach > 0.3 && rival.gap.abs() - reach < approach * OVERLAP_HORIZON + 1.0;
            let clearing = rival.gap < 0.0 && rival.gap > -(reach + PASS_CLEARANCE);
            let alongside = overlapping || in_other_lane && (arriving || clearing);
            // Only a car we are about to run into needs a lane change, and
            // only when we mean to pass it; otherwise follow it on the line
            // and let the traffic speed limit below hold us back.
            let closing = (speed - rival.speed).max(0.0);
            let close_behind_them = attacking && rival.gap > 0.0 && rival.gap < 6.0 + closing * 1.5;
            if !(alongside || close_behind_them) {
                continue;
            }
            let needed = car.spec.width + if alongside { 1.1 } else { 0.6 };
            // Keep a car's width (plus margin) away on the side we are on.
            let side = if separation > 0.0 { -1.0 } else { 1.0 };
            let bound = rival.lateral + side * needed;
            if alongside {
                // A hard boundary however far apart we are now, so the line
                // cannot drift into the car beside us.
                if side < 0.0 {
                    lane_max = lane_max.min(bound);
                } else {
                    lane_min = lane_min.max(bound);
                }
                if rival.gap > -car.spec.length * 0.3 {
                    beside_speed = beside_speed.min(rival.speed);
                }
                // Where we will be shortly, not only where we are.
                let ahead_separation = (separation - drift) * separation.signum();
                let closing_in = rival.gap.abs() < reach + 2.0
                    && separation.abs().min(ahead_separation) < needed - 0.3;
                too_close |= closing_in;
                if closing_in && rival.gap > car.spec.length * 0.2 {
                    yield_speed = yield_speed.min(rival.speed - 1.0);
                }
            }
            if separation.abs() < needed + 1.5 {
                lateral = if side < 0.0 {
                    lateral.min(bound)
                } else {
                    lateral.max(bound)
                };
            }
        }
        // Smooth only the tactical move away from the line: the line itself
        // swings across the whole road through esses and must not lag.
        let offset = lateral.clamp(-limit, limit) - line_lateral;
        self.line_offset += ((offset - self.line_offset) * (dt * 2.5).min(1.0))
            .clamp(-LANE_CHANGE_RATE * dt, LANE_CHANGE_RATE * dt);
        // Room owed to an overlapping car is a hard boundary. Smoothing the
        // safety offset otherwise cuts across that car as the line changes.
        let squeezed = lane_min > lane_max;
        let lateral_target = if !squeezed {
            (line_lateral + self.line_offset).clamp(lane_min, lane_max)
        } else {
            car.track_position.lateral.clamp(-limit, limit)
        };

        // Pure pursuit toward the target point.
        let target = if too_close {
            track
                .frame_at(s + lookahead * EVASIVE_LOOKAHEAD)
                .point_at(lateral_target)
        } else {
            ahead_frame.point_at(lateral_target)
        };
        let offset = DVec2::new(target.x - car.position.x, target.z - car.position.z);
        let local_left = offset.dot(car.left());
        let distance = offset.length().max(1.0);
        let curvature = 2.0 * local_left / (distance * distance);
        // Yaw damping steadies the line. It damps the error from the yaw rate
        // the path asks for, so a long fast corner does not bleed off lock; a
        // wall strike spikes the yaw rate far past anything a driver corrects
        // for, so it must not flick the wheel to opposite lock.
        let yaw_error = (car.yaw_rate - speed * curvature).clamp(-1.5, 1.5);
        let wheel_angle = (car.spec.wheelbase() * curvature).atan() - yaw_error * 0.03;
        let steer = (wheel_angle / car.steer_limit(speed)).clamp(-1.0, 1.0);

        // Speed: profile ahead, minus dirty-air caution, adjusted for mistakes and traffic.
        let anticipation = speed * 0.12 + 2.0;
        // A committed move outbrakes the car being passed: the attacker brakes
        // a few metres later, more so for aggressive drivers.
        let crowded = in_pack(car, rivals);
        let late_braking = match self.intent {
            Intent::Attack { side, apex_s }
                if !crowded
                    && self.attack_overlap(car, rivals)
                    && corner.is_some_and(|c| c.apex_s == apex_s && c.inside == side) =>
            {
                3.0 + 6.0 * self.profile.aggression
            }
            _ => 0.0,
        };
        // A late-braking error only applies in the braking zone it was rolled for.
        let mistake = match corner {
            Some(corner) if !crowded && self.mistake_corner == Some(corner.apex_s) => {
                let to_apex = track.delta(s, corner.apex_s).rem_euclid(track.length);
                let zone = track
                    .delta(corner.brake_s, corner.apex_s)
                    .rem_euclid(track.length);
                if to_apex < zone + 60.0 {
                    self.mistake
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        let ahead = s + anticipation - mistake - late_braking;
        let mut target_speed = match &self.launch {
            Some(launch) => launch.speed[track.station_at(ahead)],
            None => self.tactical_speed(track, ahead, on_lane),
        };
        let planned = target_speed;
        // Dirty air costs front downforce. The car is front-limited, so plan
        // for the share of front-axle load that is lost: a lot in fast
        // corners, little in slow ones where mechanical grip dominates.
        target_speed *=
            (1.0 - car.aero.dirty_air() * front_downforce_share(car, target_speed)).sqrt();
        // A bend taken flat in clean air has no speed limit to scale, so cap
        // it at what the car can corner with the downforce lost to the car
        // ahead. Ordinary corners already plan for dirty air above.
        if car.aero.dirty_air() > 0.01 {
            let path = self.launch.as_ref().map_or_else(
                || {
                    if on_lane {
                        self.tactical_line(track).unwrap_or(&track.racing_line)
                    } else {
                        &track.racing_line
                    }
                },
                |launch| &launch.line,
            );
            let bend = (0..=6)
                .map(|step| path.curvature[track.station_at(ahead + step as f64 * 8.0)].abs())
                .fold(0.0, f64::max);
            let clean = self.dirty_air_corner_limit(car, bend, 0.0);
            // The plan asks for more than steady clean-air cornering gives:
            // a flat-out bend. Scale it by what the dirty air takes away.
            if planned > FAST_BEND && planned >= clean * 0.98 && clean.is_finite() {
                let dirty = self.dirty_air_corner_limit(car, bend, car.aero.dirty_air());
                target_speed = target_speed.min(planned * dirty / clean);
            }
        }
        // Running wide with the wheel already wound on: lift until the line comes back.
        let curvature = track.racing_line.curvature[track.station_at(s)];
        let wide = (car.track_position.lateral - lateral_target) * -curvature.signum();
        if wide > 0.8 && steer.abs() > 0.9 {
            target_speed *= 1.0 - ((wide - 0.8) * 0.04).min(0.25);
        }
        if matches!(self.intent, Intent::Attack { .. }) {
            // Committed passes carry a touch more speed on the straight.
            target_speed *= 1.0 + 0.01 * self.profile.aggression;
        }
        // Held off the line by traffic or a move: brake for the bend the car
        // will actually drive, not the wider arc of the line it has left,
        // or it runs wide into the car it is racing.
        if self.launch.is_none() {
            let reference = if on_lane {
                lane.unwrap_or(&track.racing_line)
            } else {
                &track.racing_line
            };
            let steering_for = lateral_target - reference.lateral_at(s + lookahead);
            // A defender may already be well inside the line at turn-in, an
            // early apex the target no longer shows. Elsewhere the gap to the
            // line is tracking lag or a deliberate lane and is left alone.
            let now = car.track_position.lateral - reference.lateral_at(s);
            let defending = matches!(self.intent, Intent::Defend { .. });
            let deviations = std::iter::once(steering_for).chain(defending.then_some(now));
            for deviation in deviations {
                if deviation.abs() >= OFF_LINE_DEADBAND {
                    target_speed = target_speed
                        .min(self.off_line_limit(car, track, reference, on_lane, ahead, deviation));
                }
            }
        }
        // Boxed in between a car alongside and the edge of the road: the car
        // further back gives way rather than leaning on the other one.
        if squeezed && beside_speed.is_finite() {
            target_speed = target_speed.min(beside_speed - 2.0);
        }
        target_speed = target_speed.min(yield_speed);
        let unconstrained = target_speed;
        let mut held_back = false;
        let corner_side = track.racing_line.curvature[track.station_at(s + anticipation)].signum();
        for rival in rivals {
            // Half alongside on the outside of a corner without a committed
            // move: concede and tuck in behind rather than run wide.
            let outside = (car.track_position.lateral - rival.lateral) * corner_side < 0.0;
            let half_alongside =
                rival.gap > car.spec.length * 0.3 && rival.gap < car.spec.length + 0.5;
            if half_alongside && outside && self.intent == Intent::Line && corner_side != 0.0 {
                target_speed = target_speed.min(rival.speed - 1.0);
            }
            // Our lane now, or the one we are steering into.
            // A much slower car (crashed, spinning or rejoining) takes more
            // room than its width, so give it a wider berth.
            let crawling = rival.speed < speed * 0.6;
            let clear = car.spec.width + if crawling { 1.8 } else { 0.6 };
            let in_line = (rival.lateral - car.track_position.lateral).abs() < clear
                || (rival.lateral - lateral_target).abs() < clear;
            if in_line && rival.gap > 0.0 {
                // Reserve the body length and braking distance for our closing
                // speed. React to closing, not absolute speed: a steady tow
                // must still let a faster car reach a passing opportunity.
                let room = (rival.gap - car.spec.length - if crowded { 3.0 } else { 1.5 }).max(0.0);
                // A fast car can stop hard: aero braking at 300 km/h is well
                // over 30 m/s². Assuming a slow car's 8 m/s² everywhere held
                // followers a full tow's length back, too far to pull out.
                let deceleration = if crowded {
                    8.0
                } else {
                    (12.0 + speed * 0.25).min(32.0)
                };
                let grip_decel = car
                    .wheels
                    .iter()
                    .enumerate()
                    .map(|(index, wheel)| {
                        let load = wheel.load;
                        car.spec.tyre_friction
                            * car.tyres[index / 2].grip(car.setup.compound)
                            * wheel.surface.grip()
                            * load
                    })
                    .sum::<f64>()
                    / car.spec.mass
                    * 0.7;
                // Grip already spent cornering is not there for braking.
                let cornering =
                    speed * speed * track.racing_line.curvature[track.station_at(s)].abs();
                let braking_grip = (grip_decel.powi(2) - cornering.powi(2))
                    .max((grip_decel * 0.25).powi(2))
                    .sqrt();
                let deceleration = deceleration.min(braking_grip);
                let closing = (speed - rival.speed).max(0.0);
                let safe_speed =
                    (rival.speed.powi(2) + 2.0 * deceleration * room).sqrt() - closing * 0.7;
                target_speed = target_speed.min(safe_speed.max(0.0));
                // Held back by this car, or reeling it in on its tow.
                let towing = car.aero.slipstream() > 0.06 && closing > 0.2;
                held_back |= rival.gap < HELD_UP_RANGE
                    && rival.speed > 10.0
                    && (safe_speed < unconstrained - 0.3 || towing);
            }
        }
        self.held_up = if held_back {
            self.held_up + dt
        } else {
            (self.held_up - dt * 0.5).max(0.0)
        };
        let error = target_speed - speed;
        let (throttle, brake) = if error > 0.0 {
            ((error * 0.35 + 0.35).clamp(0.0, 1.0), 0.0)
        } else if error > -1.5 {
            (0.15, 0.0)
        } else {
            (0.0, ((-error - 1.5) * 0.22).clamp(0.0, 1.0))
        };
        // Feed the throttle in as the steering unwinds where traction is the
        // limit; at high speed the engine cannot overpower the rears, so fast
        // bends such as Eau Rouge stay flat. Back off if the rear slides.
        let rear_sliding = car.wheels[2].sliding.max(car.wheels[3].sliding);
        let traction_limited = ((TRACTION_SPEED - speed) / 20.0).clamp(0.0, 1.0);
        let throttle =
            throttle * (1.0 - rear_sliding * 0.6) * (1.0 - steer.abs() * 0.35 * traction_limited);

        CarInput {
            steer,
            throttle,
            brake,
            shift_up: false,
            shift_down: false,
            // The race only lets the flap open where DRS is earned.
            drs: true,
        }
    }

    fn tactical_line<'a>(&self, track: &'a Track) -> Option<&'a RacingLine> {
        let side = match self.intent {
            Intent::Line => return None,
            Intent::Attack { side, .. } | Intent::Defend { side, .. } => side,
        };
        track
            .passing_lines
            .as_ref()
            .map(|lines| &lines[usize::from(side < 0.)])
    }

    /// Seconds stuck behind a car before pulling out to pass it.
    fn patience(&self) -> f64 {
        (1.3 - 0.9 * self.profile.aggression).clamp(0.35, 1.3)
    }

    /// Speed cap from `s` on for a car `deviation` metres off `reference`.
    ///
    /// Where the reference shifted sideways, kept on the road, is tighter,
    /// the plan's own speed there is scaled by the ratio of the two bends'
    /// cornering limits. The plan only uses part of the grip the
    /// car has, so tightening within that headroom is free: a car slightly
    /// off the line through a fast direction change is not slowed. The cap
    /// then allows braking down to it with the plan's grip.
    fn off_line_limit(
        &self,
        car: &Car,
        track: &Track,
        reference: &RacingLine,
        on_lane: bool,
        s: f64,
        deviation: f64,
    ) -> f64 {
        let spec = &car.spec;
        let grip = spec.tyre_friction * GRIP_MARGIN * self.profile.pace * self.planned_grip;
        let headroom = (FLAT_MARGIN / (GRIP_MARGIN * self.profile.pace)).sqrt();
        let downforce = 0.5
            * crate::car::AIR_DENSITY
            * (car.setup.front_lift_area(&spec.aero) + car.setup.rear_lift_area(&spec.aero))
            / spec.mass;
        let point = |at: f64, shifted: bool| {
            let frame = track.frame_at(at);
            let limit = (frame.half_width - 1.4).max(0.0);
            let shift = if shifted { deviation } else { 0.0 };
            let lateral = (reference.lateral_at(at) + shift).clamp(-limit, limit);
            let p = frame.point_at(lateral);
            DVec2::new(p.x, p.z)
        };
        // Curvature over a chord long enough to average out the kink where
        // the shifted path meets the edge of the road.
        let bend = |at: f64, shifted: bool| {
            signed_curvature(
                point(at - OFF_LINE_CHORD, shifted),
                point(at, shifted),
                point(at + OFF_LINE_CHORD, shifted),
            )
            .abs()
        };
        let dirty = car.aero.dirty_air();
        let mut cap = f64::INFINITY;
        for step in 0..OFF_LINE_SAMPLES {
            let distance = step as f64 * OFF_LINE_STEP;
            let at = s + distance;
            let shifted = bend(at, true);
            let original = bend(at, false);
            if shifted <= original * 1.02 + 1e-4 {
                continue;
            }
            let tight = self.dirty_air_corner_limit(car, shifted, dirty);
            if !tight.is_finite() {
                continue;
            }
            let wide = self.dirty_air_corner_limit(car, original, dirty);
            let planned = self.tactical_speed(track, at, on_lane);
            let corner = if wide.is_finite() {
                planned * (tight / wide * headroom).min(1.0)
            } else {
                // Held by downforce on the reference at any speed: the
                // tighter bend's own limit with the car's real grip.
                (tight * headroom).min(planned)
            };
            let braking = 0.76
                * self.profile.pace
                * grip
                * (crate::car::GRAVITY + downforce * corner * corner);
            cap = cap.min((corner * corner + 2.0 * braking * distance).sqrt());
        }
        cap
    }

    fn tactical_speed(&self, track: &Track, s: f64, on_lane: bool) -> f64 {
        let optimal = self.target_speed(track, s);
        if !on_lane {
            return optimal;
        }
        let side = match self.intent {
            Intent::Line => return optimal,
            Intent::Attack { side, .. } | Intent::Defend { side, .. } => side,
        };
        // During a lane transition respect both paths' braking envelopes.
        self.passing_speeds.as_ref().map_or(optimal, |speeds| {
            optimal.min(speeds[usize::from(side < 0.)][track.station_at(s)])
        })
    }

    fn lane_at(&self, track: &Track, side: f64, s: f64) -> f64 {
        track.passing_lines.as_ref().map_or(
            side * (track.frame_at(s).half_width - 2.6).max(0.),
            |lines| lines[usize::from(side < 0.)].lateral_at(s),
        )
    }

    /// Check the swept corridor and rival closure over the time needed to change lanes.
    fn lane_is_clear(&self, car: &Car, track: &Track, rivals: &[Rival], side: f64) -> bool {
        let speed = car.speed();
        let s = car.track_position.s;
        let from = car.track_position.lateral;
        let first_lane = self.lane_at(track, side, s + speed * 0.5);
        let horizon = ((first_lane - from).abs() / LANE_CHANGE_RATE).max(1.0);
        let distance = speed * horizon;
        let steps = (distance / STATION_SPACING).ceil().max(1.0) as usize;
        let (mut min, mut max) = (from, from);
        for step in 1..=steps {
            let at = s + distance * step as f64 / steps as f64;
            let lane = self.lane_at(track, side, at);
            min = min.min(lane);
            max = max.max(lane);
        }
        min -= car.spec.width + 0.7;
        max += car.spec.width + 0.7;
        !rivals.iter().any(|rival| {
            let predicted = rival.gap + (rival.speed - speed) * horizon;
            let crosses = rival.gap.signum() != predicted.signum();
            (crosses || rival.gap.abs().min(predicted.abs()) < car.spec.length + 2.0)
                && rival.lateral > min
                && rival.lateral < max
        })
    }

    /// The side to pass `target` on, if either has room and is clear.
    /// Near the braking zone the inside comes first; on the straight, the
    /// side with more road beside the car being passed.
    fn attack_side(
        &self,
        car: &Car,
        track: &Track,
        rivals: &[Rival],
        target: &Rival,
        corner: CornerPlan,
        to_brake: f64,
    ) -> Option<f64> {
        let s = car.track_position.s;
        let half_width = track.frame_at(s + target.gap).half_width;
        let room = |side: f64| half_width - side * target.lateral;
        let first = if to_brake < 250.0 + car.speed() * 2.0
            || (room(corner.inside) - room(-corner.inside)).abs() < car.spec.width
        {
            corner.inside
        } else if room(1.0) > room(-1.0) {
            1.0
        } else {
            -1.0
        };
        [first, -first].into_iter().find(|&side| {
            let lane = self.lane_at(track, side, s + car.speed() * 0.5);
            // The lane, or alongside the target if that is further out, must
            // fit on the road and be free of other cars.
            let beside = target.lateral + side * (car.spec.width + 1.3);
            let limit = (half_width - 1.4).max(0.0);
            (lane - target.lateral) * side > car.spec.width + 0.7
                && beside * side <= limit + 0.2
                && self.lane_is_clear(car, track, rivals, side)
                && lane_is_free(car, rivals, target, beside)
        })
    }

    /// Whether the car being attacked is close enough that the move is on.
    fn attack_overlap(&self, car: &Car, rivals: &[Rival]) -> bool {
        rivals
            .iter()
            .any(|rival| rival.gap > -car.spec.length && rival.gap < car.spec.length * 1.5)
    }

    fn replan(&mut self, car: &Car, track: &Track, rivals: &[Rival], corner: Option<CornerPlan>) {
        // The whole-lap profile only needs rebuilding once the tyres have changed.
        let grip = tyre_grip(car);
        if (grip - self.planned_grip).abs() > self.planned_grip * 0.004 {
            self.set_plan(self.profile, grip, car, track);
            if let Some(launch) = self.launch.as_mut() {
                launch.state.grip = grip;
                launch.speed = speed_profile(self.profile, car, &launch.line, grip);
            }
        }
        if self.launch.is_some() {
            self.intent = Intent::Line;
            return;
        }
        let s = car.track_position.s;
        // Forget a plan once its corner is behind us.
        match self.intent {
            Intent::Attack { apex_s, .. } | Intent::Defend { apex_s, .. }
                if track.delta(s, apex_s) < -40.0 =>
            {
                self.intent = Intent::Line;
            }
            _ => {}
        }
        let Some(corner) = corner else {
            self.moved_for_corner = None;
            self.intent = Intent::Line;
            return;
        };
        if self
            .moved_for_corner
            .is_some_and(|apex| apex != corner.apex_s)
            && self.intent == Intent::Line
        {
            self.moved_for_corner = None;
        }
        let to_brake = track.delta(s, corner.brake_s).rem_euclid(track.length);
        // A packed braking zone has no free lane for an attacking or defensive
        // swerve. Hold position until the pack spreads, then resume racing.
        // On the straight before it a clear lane is still worth taking.
        let braking_zone = to_brake < 120.0 + car.speed() || to_brake > track.length * 0.5;
        if in_pack(car, rivals) && braking_zone {
            self.mistake = 0.0;
            return;
        }

        // An occasional late-braking error, decided once per corner.
        if self.mistake_corner != Some(corner.apex_s) {
            self.mistake_corner = Some(corner.apex_s);
            let chance = (1.0 - self.profile.consistency) * 0.6;
            self.mistake = if self.rng.next_f64() < chance {
                4.0 + self.rng.next_f64() * 10.0
            } else {
                0.0
            };
        }

        let ahead = rivals
            .iter()
            .filter(|rival| rival.gap > 0.0 && rival.gap < 45.0)
            .min_by(|a, b| a.gap.total_cmp(&b.gap));
        let behind = rivals
            .iter()
            .filter(|rival| rival.gap < 0.0 && rival.gap > -20.0)
            .max_by(|a, b| a.gap.total_cmp(&b.gap));

        // On tracks with passing lanes a defender holds its single move to
        // the apex; an attacker may still abort a hopeless move below.
        if track.passing_lines.is_some() && matches!(self.intent, Intent::Defend { .. }) {
            return;
        }
        match self.intent {
            Intent::Attack { apex_s, .. } => {
                // Back out of a hopeless move instead of dive-bombing.
                if let Some(target) = ahead {
                    // Committed once the noses overlap; otherwise give up at
                    // the braking point rather than dive into the back.
                    // Decide early enough to tuck back in before braking.
                    // Project the gap to the braking point at the current
                    // closing speed: give up only if the nose will not be
                    // alongside by then.
                    let closing = car.speed() - target.speed;
                    let time_to_brake = to_brake / car.speed().max(1.0);
                    let projected = target.gap - closing.max(0.0) * time_to_brake;
                    let no_overlap = projected > car.spec.length * 0.9;
                    let decision = 25.0 + car.speed() * 0.6;
                    // Out of the tow and falling back with no overlap: the
                    // move has failed, so tuck back in now rather than lose
                    // the slipstream all the way to the braking zone.
                    let fading =
                        self.pulled_out && closing < -0.3 && target.gap > car.spec.length * 1.2;
                    if no_overlap && to_brake < decision && to_brake < track.length * 0.5 {
                        self.intent = Intent::Line;
                    } else if fading {
                        // Back into the tow with the move still to make: it
                        // may bring us close enough to try again before
                        // braking, once we have been held up afresh.
                        self.intent = Intent::Line;
                        self.moved_for_corner = None;
                        self.held_up = 0.0;
                    } else if apex_s != corner.apex_s {
                        // The next corner of a chicane flips the inside: start
                        // a fresh decision there instead of carrying the side.
                        self.intent = Intent::Line;
                    } else if let Intent::Attack { side, .. } = self.intent {
                        // The car ahead has moved over onto our side (its line
                        // swings across the straight): switch to the side
                        // that has opened, while we are still behind it.
                        let limit = (track.frame_at(s + target.gap).half_width - 1.4).max(0.0);
                        let beside = target.lateral + side * (car.spec.width + 1.3);
                        let behind = target.gap > car.spec.length * 0.9;
                        if behind && beside * side > limit + 0.2 {
                            if let Some(other) =
                                self.attack_side(car, track, rivals, target, corner, to_brake)
                            {
                                self.intent = Intent::Attack {
                                    side: other,
                                    apex_s: corner.apex_s,
                                };
                            }
                        }
                    }
                } else if rivals
                    .iter()
                    .all(|rival| rival.gap.abs() > car.spec.length + 2.0)
                {
                    self.intent = Intent::Line;
                }
            }
            Intent::Defend { .. } => {
                if behind.is_none() {
                    self.intent = Intent::Line;
                }
            }
            Intent::Line => {
                if self.moved_for_corner == Some(corner.apex_s) {
                    return;
                }
                let corner_close = to_brake < 320.0;
                if let Some(target) = ahead {
                    let closing = car.speed() - target.speed;
                    let towed = car.aero.slipstream() > 0.06;
                    let keen = 0.35 + self.profile.aggression * 0.5;
                    let in_range = target.gap < 12.0 + keen * 20.0;
                    // Before a braking zone, line up a move when catching or
                    // towed; on a straight, pull out of a strong tow to pass.
                    let straight_pass =
                        (towed || car.drs.flap > 0.5 || track.passing_lines.is_some())
                            && closing > 0.5
                            && target.gap < 22.0;
                    // Stuck behind it, on the straight or into a corner: go
                    // and pass rather than wait for the leader to give way.
                    // Only set up before the braking zone: a move started at
                    // turn-in is a lunge, not a pass.
                    let before_braking =
                        to_brake > 40.0 + car.speed() * 0.6 && to_brake < track.length * 0.5;
                    let held_up =
                        self.held_up > self.patience() && closing > -0.5 && before_braking;
                    // Close enough for the move to be on: further back the
                    // tow is still bringing us up, and leaving it now wastes it.
                    let within_reach =
                        target.gap < car.spec.length * HELD_UP_ATTACK + closing.max(0.0) * 2.0;
                    if in_range && (corner_close && (towed || closing > 1.5) || straight_pass)
                        || within_reach && held_up
                    {
                        if let Some(side) =
                            self.attack_side(car, track, rivals, target, corner, to_brake)
                        {
                            self.intent = Intent::Attack {
                                side,
                                apex_s: corner.apex_s,
                            };
                            self.moved_for_corner = Some(corner.apex_s);
                            return;
                        }
                    }
                }
                if let Some(chaser) = behind {
                    let threat = chaser.speed > car.speed() - 1.0 && chaser.gap > -14.0;
                    // A car reeling us in on a long straight is covered
                    // early, while it is still in the tow: once it has pulled
                    // out alongside, moving over would be blocking, not
                    // defending.
                    let in_tow =
                        (chaser.lateral - car.track_position.lateral).abs() < car.spec.width;
                    let run_on_us = chaser.speed > car.speed() + 1.0
                        && to_brake < DEFEND_RUN_DISTANCE
                        && chaser.gap > -14.0
                        && in_tow;
                    if (corner_close || run_on_us)
                        && threat
                        && self.profile.aggression > 0.25
                        && self.lane_is_clear(car, track, rivals, corner.inside)
                    {
                        self.moved_for_corner = Some(corner.apex_s);
                        self.intent = Intent::Defend {
                            side: corner.inside,
                            apex_s: corner.apex_s,
                        };
                    }
                }
            }
        }
    }
}

/// Seconds ahead a car in another lane counts as alongside already.
const OVERLAP_HORIZON: f64 = 1.2;
/// Seconds of our own sideways drift counted when judging a car alongside.
const LATERAL_PREVIEW: f64 = 0.4;
/// Share of the steering lookahead used to get out of a squeeze.
const EVASIVE_LOOKAHEAD: f64 = 0.6;
/// Metres a passer must be clear ahead before it may cross the passed car's lane.
const PASS_CLEARANCE: f64 = 3.0;
/// Metres before a braking zone within which a defender covers a car
/// closing on it, rather than only once the braking zone is near.
const DEFEND_RUN_DISTANCE: f64 = 700.0;
/// Seconds of a faster car's approach from behind checked before pulling out.
const MIRROR_HORIZON: f64 = 1.5;
/// Car lengths behind the car ahead within which being held up starts an
/// attack, and within which it pulls the attacker out of the tow.
const HELD_UP_ATTACK: f64 = 4.0;
const HELD_UP_PULL_OUT: f64 = 2.5;
/// Metres behind a car in our lane within which it can hold us up.
const HELD_UP_RANGE: f64 = 35.0;
/// Off-line deviation (m) below which the racing-line plan still holds.
const OFF_LINE_DEADBAND: f64 = 0.8;
/// Sample spacing (m) and count for the off-line corner check.
const OFF_LINE_STEP: f64 = 6.0;
const OFF_LINE_SAMPLES: usize = 28;
/// Half-chord (m) over which the shifted and reference bends are measured.
const OFF_LINE_CHORD: f64 = 12.0;

/// Signed curvature of the circle through three ground-plane points.
fn signed_curvature(a: DVec2, b: DVec2, c: DVec2) -> f64 {
    let (ab, bc, ca) = (b - a, c - b, a - c);
    let denominator = ab.length() * bc.length() * ca.length();
    if denominator < 1e-9 {
        0.0
    } else {
        2.0 * ab.perp_dot(bc) / denominator
    }
}

/// Whether the passing lane at `lateral` is worth pulling into: no other car
/// in it ahead within range, which would just hold us up there instead, and
/// no faster car coming up behind in it that we would pull out in front of.
fn lane_is_free(car: &Car, rivals: &[Rival], target: &Rival, lateral: f64) -> bool {
    let speed = car.speed();
    !rivals.iter().any(|rival| {
        if (rival.lateral - lateral).abs() > car.spec.width + 0.8
            || rival.position == target.position
        {
            return false;
        }
        if rival.gap > 0.0 {
            rival.gap < HELD_UP_RANGE
        } else {
            let closing = (rival.speed - speed).max(0.0);
            rival.gap > -(car.spec.length + 2.0 + closing * MIRROR_HORIZON)
        }
    })
}

/// Multiple nearby cars leave no clear lane for a tactical move.
fn in_pack(car: &Car, rivals: &[Rival]) -> bool {
    rivals
        .iter()
        .filter(|rival| {
            rival.gap.abs() < 20.0
                && (rival.lateral - car.track_position.lateral).abs() < car.spec.width * 2.0
        })
        .count()
        >= 2
}

/// Planned speed (m/s, ~250 km/h) above which a bend counts as flat out
/// for the dirty-air cap; slower corners plan for dirty air directly.
const FAST_BEND: f64 = 70.0;

/// Above this speed (m/s) drive torque no longer threatens rear grip, so
/// steering lock stops trimming the throttle.
const TRACTION_SPEED: f64 = 60.0;

/// Share of tyre friction the AI plans to use at low load.
const GRIP_MARGIN: f64 = 0.88;
/// The plan's grip falls off with aero load like the tyres do, so fast
/// corners are not planned faster than the loaded tyres can hold.
const LOAD_SENSITIVITY: f64 = 0.04;

/// Match the braking plan to the tyre compound, temperature and wear used by
/// the physical car, rather than assuming optimal tyres for the whole race.
fn speed_profile(profile: DriverProfile, car: &Car, line: &RacingLine, tyre_grip: f64) -> Vec<f64> {
    let spec = &car.spec;
    let envelope = |grip: f64, brake: f64| PerformanceEnvelope {
        mass: spec.mass,
        grip,
        lift_area: car.setup.front_lift_area(&spec.aero) + car.setup.rear_lift_area(&spec.aero),
        drag_area: car.setup.drag_area(&spec.aero),
        // Slightly optimistic, so on straights and flat-out bends the car can
        // never catch the plan and the throttle saturates instead of tracking
        // the curve. Braking points come from the backward pass and are
        // unaffected.
        power: spec.engine.max_power * 1.1,
        brake_fraction: brake,
        load_sensitivity: LOAD_SENSITIVITY,
    };
    line.speed_profile_keeping_flat(
        &envelope(
            spec.tyre_friction * GRIP_MARGIN * profile.pace * tyre_grip,
            // Leave headroom for axle brake bias, ABS trimming and turn-in loads.
            0.76 * profile.pace,
        ),
        // Every driver keeps flat the bends the car itself holds flat with a
        // small margin, whatever their pace.
        &envelope(spec.tyre_friction * FLAT_MARGIN * tyre_grip, 0.76),
    )
}

fn passing_speeds(
    profile: DriverProfile,
    car: &Car,
    track: &Track,
    grip: f64,
) -> Option<[Vec<f64>; 2]> {
    track.passing_lines.as_ref().map(|lines| {
        lines
            .each_ref()
            .map(|line| speed_profile(profile, car, line, grip))
    })
}

/// Share of the car's real grip a bend must fit within to be taken flat.
const FLAT_MARGIN: f64 = 0.96;

impl AiDriver {
    /// Highest steady speed for `curvature` with this car's downforce cut by
    /// `dirty` (0 = clean air), using the same grip the plan uses. Infinite
    /// when downforce alone holds the bend at any speed.
    fn dirty_air_corner_limit(&self, car: &Car, curvature: f64, dirty: f64) -> f64 {
        let spec = &car.spec;
        let grip = spec.tyre_friction * GRIP_MARGIN * self.profile.pace * self.planned_grip;
        let lift = car.setup.front_lift_area(&spec.aero) * (1.0 - dirty)
            + car.setup.rear_lift_area(&spec.aero) * (1.0 - dirty * 0.5);
        let downforce = 0.5 * crate::car::AIR_DENSITY * lift / spec.mass;
        let denominator = curvature - grip * downforce;
        if denominator <= 1e-9 {
            f64::INFINITY
        } else {
            (grip * crate::car::GRAVITY / denominator).sqrt()
        }
    }
}

/// Fraction of the front axle's load that comes from downforce at `speed`.
fn front_downforce_share(car: &Car, speed: f64) -> f64 {
    let spec = &car.spec;
    let downforce =
        0.5 * crate::car::AIR_DENSITY * car.setup.front_lift_area(&spec.aero) * speed * speed;
    let weight = spec.mass * crate::car::GRAVITY * spec.front_weight;
    downforce / (downforce + weight)
}

fn tyre_grip(car: &Car) -> f64 {
    car.tyres
        .iter()
        .map(|tyre| tyre.grip(car.setup.compound))
        .fold(f64::INFINITY, f64::min)
}

/// Finds braking zones and apexes from the speed profile's local minima.
fn find_corners(track: &Track, speed: &[f64]) -> Vec<CornerPlan> {
    let count = speed.len();
    let window = (60.0 / STATION_SPACING) as usize;
    let mut corners = Vec::new();
    for index in 0..count {
        let here = speed[index];
        let is_minimum = (1..=window).all(|offset| {
            here <= speed[(index + offset) % count]
                && here < speed[(index + count - offset) % count]
        });
        if !is_minimum {
            continue;
        }
        // Only corners that need a real lift or brake.
        let approach_peak = (window..window * 8)
            .map(|offset| speed[(index + count - offset) % count])
            .fold(0.0_f64, f64::max);
        if approach_peak < here * 1.08 {
            continue;
        }
        let mut brake = index;
        for step in 1..window * 8 {
            let candidate = (index + count - step) % count;
            if speed[candidate] >= approach_peak * 0.995 {
                brake = candidate;
                break;
            }
        }
        let curvature = track.racing_line.curvature[index];
        corners.push(CornerPlan {
            brake_s: brake as f64 * STATION_SPACING,
            apex_s: index as f64 * STATION_SPACING,
            inside: if curvature >= 0.0 { 1.0 } else { -1.0 },
            apex_speed: here,
            passing: approach_peak > here * PASSING_SPEED_DROP,
        });
    }
    corners
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::spec::{test_wheels, CarSpec, Setup};
    use crate::car::Assists;
    use crate::track::oval_track;
    use std::sync::Arc;

    fn driver_and_car(track: &Track) -> (AiDriver, Car) {
        let (position, yaw) = track.grid_slot(0);
        let car = Car::new(
            Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            track,
            position,
            yaw,
        );
        let driver = AiDriver::new(
            DriverProfile {
                pace: 0.97,
                aggression: 0.5,
                consistency: 1.0,
            },
            &car,
            track,
            7,
        );
        (driver, car)
    }

    #[test]
    fn restoring_a_previous_grip_reuses_its_plan_instead_of_rebuilding() {
        let track = oval_track();
        let (mut driver, car) = driver_and_car(&track);
        let before = driver.snapshot();
        let original = driver.speed_profile.as_ptr();
        driver.set_plan(driver.profile, driver.planned_grip * 0.9, &car, &track);
        assert_ne!(driver.speed_profile.as_ptr(), original);
        // A rollback to the authoritative state swaps the cached plan back.
        driver.restore(&before, &car, &track);
        assert_eq!(driver.speed_profile.as_ptr(), original);
        assert_eq!(driver.planned_grip, before.planned_grip);
    }

    #[test]
    fn finds_both_ends_of_the_oval_as_corners() {
        let track = oval_track();
        let (driver, _) = driver_and_car(&track);
        assert_eq!(driver.corners.len(), 2, "{:?}", driver.corners);
        assert!(driver.corners.iter().all(|corner| corner.apex_speed > 20.0));
    }

    #[test]
    fn replanning_brakes_for_the_current_tyre_grip() {
        let track = oval_track();
        let (mut driver, mut car) = driver_and_car(&track);
        for tyre in &mut car.tyres {
            tyre.temperature = 100.0;
        }
        driver.replan(&car, &track, &[], None);
        let warm = driver.target_speed(&track, 250.0);
        for tyre in &mut car.tyres {
            tyre.temperature = 35.0;
            tyre.wear = 0.6;
        }
        driver.replan(&car, &track, &[], None);
        let cold_worn = driver.target_speed(&track, 250.0);
        assert!(
            cold_worn < warm * 0.95,
            "warm {warm}, cold/worn {cold_worn}"
        );
    }

    #[test]
    fn lane_clearance_covers_the_full_lateral_transition() {
        let track = oval_track();
        let (driver, mut car) = driver_and_car(&track);
        car.track_position.s = 100.0;
        car.track_position.lateral = -1.5;
        let forward = car.forward();
        car.velocity = forward * 50.0;

        let from = car.track_position.lateral;
        let lane = driver.lane_at(&track, 1.0, car.track_position.s + car.speed() * 0.5);
        let horizon = (lane - from).abs() / LANE_CHANGE_RATE;
        assert!(horizon > 1.0);
        let closing = 6.0;
        let danger = car.spec.length + 2.0;
        let gap = danger + closing * (horizon + 1.0) * 0.5;
        let rival = Rival {
            gap,
            lateral: (from + lane) * 0.5,
            speed: car.speed() - closing,
            position: DVec2::ZERO,
        };

        assert!(
            gap - closing > danger,
            "the rival is clear after one second"
        );
        assert!(
            gap - closing * horizon < danger,
            "the rival reaches the lane before the transition completes"
        );
        assert!(!driver.lane_is_clear(&car, &track, &[rival], 1.0));
    }

    #[test]
    fn off_line_limit_accounts_for_tighter_displaced_path() {
        let track = crate::track::stadium(200.0, 25.0, Vec::new());
        let (driver, car) = driver_and_car(&track);
        let reference = RacingLine::constant_offset(&track.stations, 0.0);
        let (station, curvature) = track
            .racing_line
            .curvature
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
            .unwrap();
        let s = station as f64 * STATION_SPACING;
        let deviation = curvature.signum() * 4.0;
        let planned = driver.target_speed(&track, s);
        let capped = driver.off_line_limit(&car, &track, &reference, false, s, deviation);
        assert!(
            capped < planned,
            "tighter displaced path was not capped: {capped:.1} vs {planned:.1} m/s"
        );
    }

    #[test]
    fn laps_the_oval_on_the_asphalt() {
        let track = oval_track();
        let (mut driver, mut car) = driver_and_car(&track);
        let dt = 1.0 / 360.0;
        let mut distance = 0.0;
        let mut previous = car.track_position.s;
        let mut worst_lateral: f64 = 0.0;
        for tick in 0..(60.0 / dt) as usize {
            let input = driver.drive(&car, &track, &[], dt);
            let t = tick as f64 * dt;
            if std::env::var("AI_FINE").is_ok() && (22.6..24.0).contains(&t) && tick % 12 == 0 {
                println!(
                    "  t={t:.2} v={:.1} r={:.3} steer={:.2} thr={:.2} brk={:.2} lat={:.2} slip={:.3},{:.3},{:.3},{:.3} fy={:.0},{:.0},{:.0},{:.0} spin={},{} gear={} rpm={:.0}",
                    car.speed(), car.yaw_rate, input.steer, input.throttle, input.brake, car.track_position.lateral,
                    car.wheels[0].slip_angle, car.wheels[1].slip_angle, car.wheels[2].slip_angle, car.wheels[3].slip_angle,
                    car.wheels[0].lateral_force, car.wheels[1].lateral_force, car.wheels[2].lateral_force, car.wheels[3].lateral_force,
                    car.wheels[2].spinning, car.wheels[3].spinning, car.powertrain.gear, car.powertrain.rpm
                );
            }
            if std::env::var("AI_PROBE").is_ok() && tick % 180 == 0 {
                println!(
                    "t={:.1} s={:.0} v={:.1} target={:.1} thr={:.2} brk={:.2} steer={:.2} lat={:.2} gear={} slide={:.2}",
                    tick as f64 * dt, car.track_position.s, car.speed(),
                    driver.target_speed(&track, car.track_position.s), input.throttle, input.brake,
                    input.steer, car.track_position.lateral, car.powertrain.gear, car.wheels[2].sliding
                );
            }
            car.step(&input, &track, dt);
            distance += track.delta(previous, car.track_position.s);
            previous = car.track_position.s;
            if distance > 50.0 {
                worst_lateral = worst_lateral.max(car.track_position.lateral.abs());
            }
        }
        assert!(
            distance > track.length * 2.0,
            "only {distance} m in a minute"
        );
        assert!(worst_lateral < 6.5, "left the track: {worst_lateral} m");
    }
}

/// Needs converted assets: `F1_ASSETS=assets cargo test -p racecraft-sim -- --ignored eau_rouge`.
#[cfg(test)]
mod eau_rouge {
    use super::*;
    use crate::car::spec::{test_wheels, CarSpec, Setup};
    use crate::car::Assists;
    use std::sync::Arc;

    /// Kemmel run-up from La Source exit to the top of Raidillon.
    const SECTION: std::ops::Range<f64> = 880.0..1250.0;

    fn spa() -> Option<Track> {
        let root = std::env::var("F1_ASSETS").ok()?;
        Some(
            Track::load(std::path::Path::new(&root).join("tracks/spa"))
                .expect("F1_ASSETS must contain a valid converted Spa track"),
        )
    }

    fn car(track: &Track) -> Car {
        let (position, yaw) = track.grid_slot(0);
        Car::new(
            Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            track,
            position,
            yaw,
        )
    }

    const PROFILE: DriverProfile = DriverProfile {
        pace: 1.0,
        aggression: 0.5,
        consistency: 1.0,
    };

    #[test]
    #[ignore]
    fn plan_keeps_accelerating_through_eau_rouge() {
        let Some(track) = spa() else { return };
        let car = car(&track);
        // Slowest grid pace, the player's guidance pace, a full-pace driver;
        // warm race tyres sit around 92-95% of peak grip.
        for pace in [0.85, 0.9, 1.0] {
            let profile = DriverProfile { pace, ..PROFILE };
            let speed = speed_profile(profile, &car, &track.racing_line, 0.92);
            let stations: Vec<usize> = (300..1400)
                .map(|s| (s as f64 / STATION_SPACING) as usize)
                .collect();
            for pair in stations.windows(2) {
                assert!(
                    speed[pair[1]] >= speed[pair[0]] - 1e-6,
                    "pace {pace}: plan lifts at {:.0} m",
                    pair[1] as f64 * STATION_SPACING
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn ai_never_brakes_through_eau_rouge_on_two_successive_laps() {
        let Some(track) = spa() else { return };
        for pace in [0.85, 1.0] {
            drive_eau_rouge(&track, DriverProfile { pace, ..PROFILE });
        }
    }

    fn drive_eau_rouge(track: &Track, profile: DriverProfile) {
        let mut car = car(track);
        let mut driver = AiDriver::new(profile, &car, track, 7);
        let dt = 1.0 / 360.0;
        let (mut lap, mut last_s) = (0, car.track_position.s);
        let mut worst = [(1.0_f64, 0.0_f64); 2];
        let mut samples = [0; 2];
        for _ in 0..(240.0 / dt) as usize {
            let input = driver.drive(&car, track, &[], dt);
            car.step(&input, track, dt);
            let s = car.track_position.s;
            if s + 1000.0 < last_s {
                lap += 1;
            }
            last_s = s;
            if (1..=2).contains(&lap) && SECTION.contains(&s) {
                samples[lap - 1] += 1;
                let (throttle, brake) = &mut worst[lap - 1];
                *throttle = throttle.min(input.throttle);
                *brake = brake.max(input.brake);
            }
            if lap == 3 {
                break;
            }
        }
        for (lap, (throttle, brake)) in worst.into_iter().enumerate() {
            assert!(samples[lap] > 0, "lap {} never reached Eau Rouge", lap + 1);
            assert!(
                brake == 0.0 && throttle > 0.99,
                "pace {}, lap {}: throttle {throttle:.2}, brake {brake:.2}",
                profile.pace,
                lap + 1
            );
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AiState {
    on_grid: bool,
    #[serde(with = "crate::exact")]
    launch: Option<LaunchState>,
    #[serde(with = "crate::exact")]
    profile: DriverProfile,
    intent: Intent,
    rng: Rng,
    mistake: f64,
    mistake_corner: Option<f64>,
    plan_timer: f64,
    stuck_timer: f64,
    line_offset: f64,
    /// One tactical move for this braking zone, even if a threat disappears.
    moved_for_corner: Option<f64>,
    #[serde(default)]
    held_up: f64,
    #[serde(default)]
    pulled_out: bool,
    #[serde(with = "crate::exact")]
    planned_grip: f64,
}
impl AiDriver {
    pub fn snapshot(&self) -> AiState {
        AiState {
            on_grid: self.on_grid,
            launch: self.launch.as_ref().map(|launch| launch.state),
            profile: self.profile,
            intent: self.intent,
            rng: self.rng.clone(),
            mistake: self.mistake,
            mistake_corner: self.mistake_corner,
            plan_timer: self.plan_timer,
            stuck_timer: self.stuck_timer,
            line_offset: self.line_offset,
            moved_for_corner: self.moved_for_corner,
            held_up: self.held_up,
            pulled_out: self.pulled_out,
            planned_grip: self.planned_grip,
        }
    }
    pub fn restore(&mut self, state: &AiState, car: &Car, track: &Track) {
        self.set_plan(state.profile, state.planned_grip, car, track);
        self.on_grid = state.on_grid;
        if self.launch.as_ref().map(|launch| launch.state) != state.launch {
            self.launch = state.launch.map(|state| {
                let line = RacingLine::constant_offset(&track.stations, state.lane);
                Launch {
                    speed: speed_profile(state.profile, car, &line, state.grip),
                    line,
                    state,
                }
            });
        }
        self.intent = state.intent;
        self.rng = state.rng.clone();
        self.mistake = state.mistake;
        self.mistake_corner = state.mistake_corner;
        self.plan_timer = state.plan_timer;
        self.stuck_timer = state.stuck_timer;
        self.line_offset = state.line_offset;
        self.moved_for_corner = state.moved_for_corner;
        self.held_up = state.held_up;
        self.pulled_out = state.pulled_out;
    }
}
