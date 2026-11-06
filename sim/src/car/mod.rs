//! Vehicle model: a planar rigid body on four tyre contact patches that follow
//! the real track surface for height, slope and grip.

pub mod powertrain;
pub mod spec;
pub mod tyre;

use crate::track::{Surface, Track, TrackPosition};
use glam::{DVec2, DVec3};
use powertrain::Powertrain;
use serde::{Deserialize, Serialize};
use spec::{CarSpec, Setup};
use std::sync::Arc;
use tyre::{lateral_shape, lateral_shape_inverse, load_sensitivity, AxleTyres};

/// Share of rear grip ABS keeps free of braking for cornering stability.
const REAR_ABS_HEADROOM: f64 = 0.35;

pub const AIR_DENSITY: f64 = 1.225;
pub const GRAVITY: f64 = 9.81;
/// Stability aid for AI-controlled cars: retain slide recovery without
/// enabling the player's yaw ceiling at the 0.5 activation boundary.
pub(crate) const AI_STABILITY: f64 = 0.49;

/// One tick of driver commands. Steering is positive to the left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CarInput {
    pub steer: f64,
    pub throttle: f64,
    pub brake: f64,
    pub shift_up: bool,
    pub shift_down: bool,
    /// Asks to open DRS; it opens only where the race allows it.
    #[serde(default)]
    pub drs: bool,
}

/// Driving aids. They help with control, never with racecraft, and are
/// deliberately a little worse than perfect technique.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assists {
    /// Steering smoothing and a speed-sensitive lock limit, 0..1.
    pub steering: f64,
    /// Partial anti-lock, 0..1.
    pub braking: f64,
    /// Traction smoothing, 0..1.
    pub traction: f64,
    /// Slide recovery, 0..1. At least 0.5 with steering above 0.5 also
    /// limits turn-in yaw overshoot; the ceiling is enabled fully rather than
    /// blended.
    pub stability: f64,
    pub automatic_gears: bool,
    /// Lets this driver open DRS anywhere, not only where it is earned.
    #[serde(default)]
    pub drs_anywhere: bool,
    /// Opens DRS by itself whenever it is available; it still closes on
    /// braking or at the end of the zone. Saved aids from before default on.
    #[serde(default = "enabled")]
    pub auto_drs: bool,
}

fn enabled() -> bool {
    true
}

impl Default for Assists {
    fn default() -> Self {
        Self {
            steering: 1.0,
            braking: 0.6,
            traction: 0.6,
            stability: 0.5,
            automatic_gears: false,
            drs_anywhere: false,
            auto_drs: false,
        }
    }
}

/// The rear-wing flap. The race sets `available`; the driver opens it, and
/// braking or leaving the zone closes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Drs {
    pub available: bool,
    pub open: bool,
    /// Flap position, 0 closed to 1 fully open.
    pub flap: f64,
}

impl Drs {
    fn update(&mut self, request: bool, brake: f64, dt: f64) {
        if !self.available || brake > 0.0 {
            self.open = false;
        } else if request {
            self.open = true;
        }
        let target = if self.open { 1.0 } else { 0.0 };
        let rate = dt / crate::drs::ACTUATION;
        self.flap += (target - self.flap).clamp(-rate, rate);
    }
}

/// How nearby cars change this car's airflow this tick.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AeroInfluence {
    /// Multiplier on drag (slipstream lowers it).
    pub drag_scale: f64,
    /// Multipliers on downforce (dirty air lowers them, the front more).
    pub front_downforce_scale: f64,
    pub rear_downforce_scale: f64,
}

impl Default for AeroInfluence {
    fn default() -> Self {
        Self {
            drag_scale: 1.0,
            front_downforce_scale: 1.0,
            rear_downforce_scale: 1.0,
        }
    }
}

impl AeroInfluence {
    pub fn slipstream(&self) -> f64 {
        1.0 - self.drag_scale
    }

    pub fn dirty_air(&self) -> f64 {
        1.0 - self.front_downforce_scale
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WheelState {
    pub load: f64,
    pub slip_angle: f64,
    /// 0 = gripping, 1 = fully sliding.
    pub sliding: f64,
    pub locked: bool,
    pub spinning: bool,
    pub surface: Surface,
    pub ground_height: f64,
    /// Accumulated rotation for rendering, rad.
    pub rotation: f64,
    /// Lateral force after tyre relaxation, N.
    pub lateral_force: f64,
}

impl Default for WheelState {
    fn default() -> Self {
        Self {
            load: 0.0,
            slip_angle: 0.0,
            sliding: 0.0,
            locked: false,
            spinning: false,
            surface: Surface::Asphalt,
            ground_height: 0.0,
            rotation: 0.0,
            lateral_force: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Car {
    pub spec: Arc<CarSpec>,
    pub setup: Setup,
    pub assists: Assists,
    /// Centre of gravity projected onto the ground.
    pub position: DVec3,
    /// Heading; forward is (sin yaw, 0, cos yaw).
    pub yaw: f64,
    pub yaw_rate: f64,
    /// Ground-plane velocity (x, z).
    pub velocity: DVec2,
    /// Visual body attitude, rad (pitch positive nose-up, roll positive left-side-up).
    pub pitch: f64,
    pub roll: f64,
    /// Visual height of the model origin, following the ground through the suspension.
    pub body_height: f64,
    suspension: [Suspension; 3],
    /// Road wheel angle, rad, positive left.
    pub steer_angle: f64,
    pub powertrain: Powertrain,
    /// Front and rear axle tyres.
    pub tyres: [AxleTyres; 2],
    pub wheels: [WheelState; 4],
    pub aero: AeroInfluence,
    pub drs: Drs,
    pub downforce: f64,
    pub track_position: TrackPosition,
    /// Smoothed body-frame acceleration (forward, left), m/s².
    pub acceleration: DVec2,
    vertical_velocity: f64,
    vertical_acceleration: f64,
    /// Strongest impact this tick (N·s), for camera shake and damage later.
    pub impact: f64,
}

/// Visual suspension: an alpha-beta tracker that follows steady slopes without
/// lag but filters the facet-to-facet noise of the collision mesh, which would
/// otherwise rock the body (and the camera with it) at ~15 Hz.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct Suspension {
    value: f64,
    rate: f64,
}

impl Suspension {
    /// Natural frequency (rad/s) and damping ratio of the body motion.
    const FREQUENCY: f64 = 20.0;
    const DAMPING: f64 = 0.75;

    fn at(value: f64) -> Self {
        Self { value, rate: 0.0 }
    }

    fn track(&mut self, target: f64, dt: f64) -> f64 {
        let alpha = (2.0 * Self::DAMPING * Self::FREQUENCY * dt).min(1.0);
        let beta = (Self::FREQUENCY * dt).powi(2).min(1.0);
        let predicted = self.value + self.rate * dt;
        let residual = target - predicted;
        self.value = predicted + alpha * residual;
        self.rate += beta * residual / dt;
        self.value
    }
}

impl Car {
    pub fn new(
        spec: Arc<CarSpec>,
        setup: Setup,
        assists: Assists,
        track: &Track,
        position: DVec3,
        yaw: f64,
    ) -> Self {
        let track_position = track.project(position, None);
        Self {
            powertrain: Powertrain::new(&spec.engine),
            spec,
            setup: setup.clamped(),
            assists,
            position,
            yaw,
            yaw_rate: 0.0,
            velocity: DVec2::ZERO,
            pitch: 0.0,
            roll: 0.0,
            body_height: position.y,
            suspension: [
                Suspension::at(position.y),
                Suspension::at(0.0),
                Suspension::at(0.0),
            ],
            steer_angle: 0.0,
            tyres: [AxleTyres::default(); 2],
            wheels: [WheelState {
                ground_height: position.y,
                ..WheelState::default()
            }; 4],
            aero: AeroInfluence::default(),
            drs: Drs::default(),
            downforce: 0.0,
            track_position,
            acceleration: DVec2::ZERO,
            vertical_velocity: 0.0,
            vertical_acceleration: 0.0,
            impact: 0.0,
        }
    }

    pub fn forward(&self) -> DVec2 {
        DVec2::new(self.yaw.sin(), self.yaw.cos())
    }

    pub fn left(&self) -> DVec2 {
        let forward = self.forward();
        DVec2::new(forward.y, -forward.x)
    }

    pub fn speed(&self) -> f64 {
        self.velocity.length()
    }

    pub fn forward_speed(&self) -> f64 {
        self.velocity.dot(self.forward())
    }

    /// World position of a model-space point (model origin on the ground
    /// between the axles), ignoring pitch and roll.
    pub fn model_to_world(&self, local: DVec3) -> DVec3 {
        let forward = self.forward();
        let left = self.left();
        let along = local.z - self.spec.cg_z();
        let flat = DVec2::new(self.position.x, self.position.z) + forward * along + left * local.x;
        DVec3::new(flat.x, self.body_height + local.y, flat.y)
    }

    /// Model origin in world space, for rendering.
    pub fn model_origin(&self) -> DVec3 {
        self.model_to_world(DVec3::ZERO)
    }

    /// Largest road wheel angle the steering assist allows at `speed`.
    pub fn steer_limit(&self, speed: f64) -> f64 {
        let (kinematic, _) = self.limit_angles(speed);
        // Lets the driver take the front tyres just past their peak: enough to
        // feel understeer when overdriving, not enough to make full lock correct.
        self.assisted_lock(kinematic + self.spec.peak_slip_angle * 1.15)
    }

    /// Lock the steering assist allows into the turn the car is already
    /// making. In a steady turn the road wheel angle is the kinematic angle
    /// plus the front slip minus the rear slip, so the full `steer_limit`
    /// runs the fronts near twice their peak slip at the limit, where the yaw
    /// response at speed overshoots and sways before it settles. Opposite
    /// lock keeps the whole `steer_limit` to catch a slide.
    fn turn_in_limit(&self, speed: f64) -> f64 {
        let (kinematic, rear_slip) = self.limit_angles(speed);
        self.assisted_lock(kinematic + self.spec.peak_slip_angle * 1.15 - rear_slip)
    }

    /// Kinematic road wheel angle of the tightest steady turn at `speed`, and
    /// the rear slip angle in it.
    fn limit_angles(&self, speed: f64) -> (f64, f64) {
        let spec = &self.spec;
        let lift = self.setup.front_lift_area(&spec.aero) + self.setup.rear_lift_area(&spec.aero);
        // Compressions such as Eau Rouge press the tyres down and allow more lock.
        let gravity = (GRAVITY + self.vertical_acceleration).clamp(GRAVITY * 0.3, GRAVITY * 2.5);
        let dynamic = 0.5 * AIR_DENSITY * speed * speed;
        let lateral_grip = spec.tyre_friction * (gravity + dynamic * lift / spec.mass);
        let kinematic = (spec.wheelbase() * lateral_grip / speed.max(1.0).powi(2)).atan();
        // The rear carries the share of the cornering force its distance from
        // the centre of gravity sets, against its static load and downforce.
        let rear_load = spec.mass * gravity * (1.0 - spec.front_weight)
            + dynamic * self.setup.rear_lift_area(&spec.aero);
        let rear_share = spec.mass * lateral_grip * (1.0 - spec.front_weight)
            / (spec.tyre_friction * spec.rear_grip * rear_load);
        (
            kinematic,
            spec.peak_slip_angle * lateral_shape_inverse(rear_share),
        )
    }

    fn assisted_lock(&self, angle: f64) -> f64 {
        const FULL_LOCK: f64 = 0.36;
        // Car::assists is mutable and can be restored from a host snapshot.
        let steering = self.assists.steering.clamp(0.0, 1.0);
        FULL_LOCK + (angle.min(FULL_LOCK) - FULL_LOCK) * steering
    }

    /// Lock the steering and stability aids allow into the turn the car is
    /// already making, so the yaw rate closes on the car's grip ceiling
    /// without overshooting it; `None` when the aid is off.
    ///
    /// Turned in at the limit, the front tyres build force at once but the
    /// rear only as the body takes up sideslip, so the yaw rate overshoots
    /// the steady turn by a third or more and swings back below it. The
    /// ceiling is the steady turn of the weaker axle at its peak: from
    /// `a·Ff = b·Fr`, `ay = Ff·L/(b·m)` for the front (`Fr·L/(a·m)` for the
    /// rear) and the yaw rate is `ay/u`. The aid asks the front for
    /// `Ff = (b·Fr + I·k·(r_max - r))/a`, the force that brings the yaw
    /// rate onto the ceiling at the rate `k`, and finds its slip angle on
    /// the tyre curve about the direction the front axle travels, never
    /// past the peak. It only takes lock away, and never into opposite lock.
    ///
    /// The ceiling requires steering above half and stability at least half.
    /// Blending the lock limit leaves the fronts beyond their peak and causes
    /// sustained yaw wobble. Lower settings retain slide recovery only.
    fn yaw_ceiling_lock(
        &self,
        target: f64,
        loads: &[f64; 4],
        forward_speed: f64,
        lateral_speed: f64,
    ) -> Option<f64> {
        // The steady-turn estimate leaves out drag, drive and kerb moments;
        // steady skidpad yaw rates stay within 4% of it from 150 to 300 km/h.
        const MARGIN: f64 = 1.05;
        // Yaw rate closing rate, 1/s: a quarter period of the ~3.5 Hz yaw
        // mode the car has with its fronts saturated. Slower costs steady
        // grip; faster lets the yaw rate overshoot again.
        const CLOSING_RATE: f64 = 15.0;
        let into_turn = target.signum();
        if self.assists.steering <= 0.5
            || self.assists.stability < 0.5
            || forward_speed <= 5.0
            || target == 0.0
            || target * self.yaw_rate < 0.0
        {
            return None;
        }
        let spec = &self.spec;
        let nominal_load = spec.mass * GRAVITY * 0.25;
        let capacity = |axle: usize, scale: f64| -> f64 {
            (2 * axle..2 * axle + 2)
                .map(|index| {
                    spec.tyre_friction
                        * scale
                        * self.tyres[axle].grip(self.setup.compound)
                        * self.wheels[index].surface.grip()
                        * load_sensitivity(loads[index], nominal_load)
                        * loads[index]
                })
                .sum()
        };
        let front = capacity(0, 1.0);
        let rear = capacity(1, spec.rear_grip);
        let wheelbase = spec.wheelbase();
        let a = spec.wheels[0].position.z - spec.cg_z();
        let b = wheelbase - a;
        let ceiling = (front / b).min(rear / a) * wheelbase / spec.mass / forward_speed * MARGIN;
        let rear_force = into_turn * (self.wheels[2].lateral_force + self.wheels[3].lateral_force);
        let wanted = (b * rear_force
            + spec.yaw_inertia * CLOSING_RATE * (ceiling - into_turn * self.yaw_rate))
            / a;
        let slip = if wanted > 0.0 {
            lateral_shape_inverse(wanted / front.max(1.0)) * spec.peak_slip_angle
        } else {
            0.0
        };
        let front_path = (lateral_speed + self.yaw_rate * a).atan2(forward_speed);
        Some((into_turn * front_path + slip).max(0.0))
    }

    pub fn step(&mut self, input: &CarInput, track: &Track, dt: f64) {
        self.step_with_power(input, track, dt, 1.);
    }

    /// Race-owned AI handicap. Human inputs always use full engine output.
    pub(crate) fn step_with_power(&mut self, input: &CarInput, track: &Track, dt: f64, power: f64) {
        let spec = self.spec.clone();
        let forward = self.forward();
        let left = self.left();
        let forward_speed = self.velocity.dot(forward);
        let lateral_speed = self.velocity.dot(left);
        let speed = self.velocity.length();
        let rear_radius = spec.wheels[2].radius;
        self.impact = 0.0;

        // Gearbox.
        if input.shift_up {
            self.powertrain.shift_up(&spec.engine, forward_speed);
        }
        if input.shift_down {
            self.powertrain
                .shift_down(&spec.engine, forward_speed, rear_radius);
        }
        if self.assists.automatic_gears {
            if self.powertrain.gear == powertrain::NEUTRAL && input.throttle > 0.1 {
                self.powertrain.shift_up(&spec.engine, forward_speed);
            }
            self.powertrain.auto_shift(
                &spec.engine,
                forward_speed,
                rear_radius,
                input.throttle,
                input.brake,
            );
        }
        let throttle = input.throttle.clamp(0.0, 1.0);
        let brake = input.brake.clamp(0.0, 1.0);
        let drive =
            self.powertrain
                .drive_force(&spec.engine, throttle, forward_speed, rear_radius, dt);

        // Preserve engine braking; scale only delivered propulsion.
        let drive = if drive > 0. { drive * power } else { drive };

        // Aerodynamics. An open DRS flap stalls part of the rear wing.
        self.drs
            .update(input.drs || self.assists.auto_drs, brake, dt);
        let dynamic_pressure = 0.5 * AIR_DENSITY * speed * speed;
        let front_downforce = dynamic_pressure
            * self.setup.front_lift_area(&spec.aero)
            * self.aero.front_downforce_scale;
        let rear_downforce = dynamic_pressure
            * (self.setup.rear_lift_area(&spec.aero) - spec.aero.drs_lift * self.drs.flap)
            * self.aero.rear_downforce_scale;
        self.downforce = front_downforce + rear_downforce;
        let drag = dynamic_pressure
            * (self.setup.drag_area(&spec.aero) - spec.aero.drs_drag * self.drs.flap)
            * self.aero.drag_scale;

        // Vertical loads with quasi-static weight transfer.
        let weight =
            spec.mass * (GRAVITY + self.vertical_acceleration).clamp(GRAVITY * 0.3, GRAVITY * 2.5);
        let wheelbase = spec.wheelbase();
        let longitudinal_transfer = spec.mass * self.acceleration.x * spec.cg_height / wheelbase;
        let front_axle =
            (weight * spec.front_weight + front_downforce - longitudinal_transfer).max(0.0);
        let rear_axle =
            (weight * (1.0 - spec.front_weight) + rear_downforce + longitudinal_transfer).max(0.0);
        let front_track = spec.wheels[0].position.x - spec.wheels[1].position.x;
        let rear_track = spec.wheels[2].position.x - spec.wheels[3].position.x;
        let lateral_force = spec.mass * self.acceleration.y * spec.cg_height;
        // A stiffer front roll setup keeps the car stable (mild understeer) at the limit.
        let front_shift = lateral_force * 0.58 / front_track;
        let rear_shift = lateral_force * 0.42 / rear_track;
        // Accelerating left loads the right-hand (outside) wheels.
        let loads = [
            (front_axle * 0.5 - front_shift).max(0.0),
            (front_axle * 0.5 + front_shift).max(0.0),
            (rear_axle * 0.5 - rear_shift).max(0.0),
            (rear_axle * 0.5 + rear_shift).max(0.0),
        ];
        let nominal_load = spec.mass * GRAVITY * 0.25;

        // Steering: smoothed toward a speed-limited target, after the loads so
        // the yaw ceiling sees the grip this tick's downforce and transfer give.
        let mut target = input.steer.clamp(-1.0, 1.0) * self.steer_limit(speed);
        if target * self.yaw_rate >= 0.0 {
            let limit = self.turn_in_limit(speed);
            target = target.clamp(-limit, limit);
        }
        let response = 30.0 - 18.0 * self.assists.steering;
        self.steer_angle += (target - self.steer_angle) * (dt * response).min(1.0);
        if let Some(lock) = self.yaw_ceiling_lock(target, &loads, forward_speed, lateral_speed) {
            let into_turn = target.signum();
            if into_turn * self.steer_angle > lock {
                self.steer_angle = into_turn * lock;
            }
        }

        let cg = DVec2::new(self.position.x, self.position.z);
        let cg_z = spec.cg_z();
        let mut force = DVec2::ZERO;
        let mut yaw_moment = 0.0;
        let mut slide_power = [0.0; 2];
        let mut rolling_power = [0.0; 2];
        let mut surface_normal = DVec3::ZERO;

        // First pass: what each contact patch stands on, how it slips, and how
        // much grip is left for braking or driving once cornering takes its share.
        struct Contact {
            surface: Surface,
            ground_height: f64,
            available: f64,
            wheel_long: f64,
            wheel_lat: f64,
            slip_angle: f64,
            lateral: f64,
            longitudinal_room: f64,
            sin_delta: f64,
            cos_delta: f64,
        }
        let contacts: [Contact; 4] = std::array::from_fn(|index| {
            let wheel_spec = &spec.wheels[index];
            let front = index < 2;
            let axle = if front { 0 } else { 1 };
            let lx = wheel_spec.position.z - cg_z;
            let ly = wheel_spec.position.x;
            let contact = cg + forward * lx + left * ly;
            let ground = track.collision.ground(DVec3::new(
                contact.x,
                self.wheels[index].ground_height + 1.5,
                contact.y,
            ));
            let (surface, ground_height, normal) = match ground {
                Some(hit) => (hit.surface, hit.height, hit.normal),
                None => (Surface::Grass, self.wheels[index].ground_height, DVec3::Y),
            };
            surface_normal += normal;
            let load = loads[index];
            let friction = spec.tyre_friction
                * if front { 1.0 } else { spec.rear_grip }
                * self.tyres[axle].grip(self.setup.compound)
                * surface.grip()
                * load_sensitivity(load, nominal_load);
            let available = friction * load;

            // Contact patch velocity in the body frame, then in the wheel frame.
            let point_forward = forward_speed - self.yaw_rate * ly;
            let point_left = lateral_speed + self.yaw_rate * lx;
            let delta = if front { self.steer_angle } else { 0.0 };
            let (sin_delta, cos_delta) = delta.sin_cos();
            let wheel_long = point_forward * cos_delta + point_left * sin_delta;
            let wheel_lat = -point_forward * sin_delta + point_left * cos_delta;
            let slip_angle = wheel_lat.atan2(wheel_long.abs().max(3.0));
            let target =
                -slip_angle.signum() * available * lateral_shape(slip_angle / spec.peak_slip_angle);
            // Relaxation: lateral force builds over ~0.4 m of rolling, not instantly.
            let relaxation = (wheel_long.abs().max(4.0) * dt / 0.4).min(1.0);
            let previous = self.wheels[index].lateral_force;
            let lateral = previous + (target - previous) * relaxation;
            let longitudinal_room = (available * available - lateral * lateral * 0.85)
                .max(available * available * 0.15)
                .sqrt();
            Contact {
                surface,
                ground_height,
                available,
                wheel_long,
                wheel_lat,
                slip_angle,
                lateral,
                longitudinal_room,
                sin_delta,
                cos_delta,
            }
        });

        // Pedal assists keep demand near what the tyres can take right now,
        // like the steering limit: lower assist levels let the driver overdrive.
        let total_grip: f64 = contacts.iter().map(|contact| contact.available).sum();
        let total_room: f64 = contacts
            .iter()
            .map(|contact| contact.longitudinal_room)
            .sum();
        // The differential delivers equal torque to both rears, so the tyre with
        // less room sets the limit; equal torque also keeps power from yawing the car.
        let rear_room = contacts[2]
            .longitudinal_room
            .min(contacts[3].longitudinal_room)
            * 2.0;
        let brake_force_total = (brake * spec.max_brake_force)
            .min(total_room * (1.0 + 0.4 * (1.0 - self.assists.braking)) + 1.0);
        let traction_cap = rear_room * (1.0 + 0.4 * (1.0 - self.assists.traction)) + 1.0;
        let drive = drive.clamp(-traction_cap, traction_cap);

        let mut wheel_brakes: [f64; 4] = std::array::from_fn(|index| {
            brake_force_total
                * if index < 2 {
                    spec.brake_bias
                } else {
                    1. - spec.brake_bias
                }
                * 0.5
        });
        if self.assists.braking >= 0.999 {
            let mut released = 0.;
            for index in 2..4 {
                let room = contacts[index].longitudinal_room;
                if wheel_brakes[index] > room {
                    let trimmed = room * (1. - REAR_ABS_HEADROOM);
                    released += wheel_brakes[index] - trimmed;
                    wheel_brakes[index] = trimmed;
                }
            }
            // Recover braking at the front while retaining its lateral reserve.
            let spare: [f64; 2] = std::array::from_fn(|index| {
                (contacts[index].longitudinal_room * 0.94 - wheel_brakes[index]).max(0.)
            });
            let extra = released.min(spare.iter().sum());
            let total: f64 = spare.iter().sum();
            if total > 0. {
                for index in 0..2 {
                    wheel_brakes[index] += extra * spare[index] / total;
                }
            }
        }

        for (index, wheel_spec) in spec.wheels.iter().enumerate() {
            let front = index < 2;
            let axle = if front { 0 } else { 1 };
            let lx = wheel_spec.position.z - cg_z;
            let ly = wheel_spec.position.x;
            let contact = &contacts[index];
            let (available, room) = (contact.available, contact.longitudinal_room);
            let wheel_long = contact.wheel_long;

            // Longitudinal demand: drive on the rear, brakes everywhere.
            let mut brake_force = wheel_brakes[index];
            let mut drive_force = if front { 0.0 } else { drive * 0.5 };

            // Partial ABS trims a locking demand to slightly under the limit, so
            // it never beats a driver who brakes right at the limit.
            // Full ABS never lets a wheel lock: one locked front tyre loses its
            // cornering force while the other three keep theirs, and the car
            // spins toward it with the lightest steering under braking.
            let mut locked = false;
            let abs = self.assists.braking.clamp(0.0, 1.0);
            let lock_margin = if abs >= 0.999 {
                f64::INFINITY
            } else {
                1.0 + 0.5 * abs / (1.0 - abs)
            };
            if brake_force > room {
                if brake_force > room * lock_margin + 50.0 {
                    locked = wheel_long.abs() > 0.5;
                } else {
                    // The rear is held further below its limit so it keeps
                    // cornering grip as weight comes off it under braking.
                    let headroom = if front { 0.06 } else { REAR_ABS_HEADROOM };
                    brake_force = room * (1.0 - headroom * abs);
                }
            }
            // Traction smoothing, strongest at low speed.
            let mut spinning = false;
            if drive_force.abs() > room {
                let assist = self.assists.traction * (1.0 - (speed / 60.0).min(1.0) * 0.5);
                if drive_force.abs() > room * (1.0 + 0.8 * assist) + 50.0 {
                    spinning = true;
                } else {
                    drive_force = drive_force.signum() * room * (1.0 - 0.05 * assist);
                }
            }

            let (fx, fy, slip_speed) = if locked {
                // A locked tyre slides along its own velocity and barely steers.
                let fx = -wheel_long.signum() * available * 0.82;
                (fx, contact.lateral * 0.3, wheel_long.abs())
            } else if spinning {
                let fx = drive_force.signum() * available * 0.8;
                (fx, contact.lateral * 0.4, (speed * 0.15).max(4.0))
            } else {
                let brake_fx = -brake_force * (wheel_long / 0.5).clamp(-1.0, 1.0);
                let fx = (drive_force + brake_fx).clamp(-available, available);
                let lateral_room = (available * available - fx * fx).max(0.0).sqrt();
                (fx, contact.lateral.clamp(-lateral_room, lateral_room), 0.0)
            };
            let rolling =
                -contact.surface.rolling_drag() * loads[index] * wheel_long.clamp(-1.0, 1.0);
            let fx = fx + rolling;

            let body_x = fx * contact.cos_delta - fy * contact.sin_delta;
            let body_y = fx * contact.sin_delta + fy * contact.cos_delta;
            force += forward * body_x + left * body_y;
            yaw_moment += lx * body_y - ly * body_x;

            let sliding = (contact.slip_angle.abs() / spec.peak_slip_angle - 1.0).clamp(0.0, 1.0);
            slide_power[axle] += fy.abs() * contact.wheel_lat.abs() + fx.abs() * slip_speed;
            rolling_power[axle] += loads[index] * speed;

            let state = &mut self.wheels[index];
            state.load = loads[index];
            state.slip_angle = contact.slip_angle;
            state.sliding = if locked || spinning { 1.0 } else { sliding };
            state.locked = locked;
            state.spinning = spinning;
            state.surface = contact.surface;
            state.ground_height = contact.ground_height;
            state.lateral_force = fy;
            if !locked {
                let spin = if spinning { 1.25 } else { 1.0 };
                state.rotation = (state.rotation + wheel_long * spin / wheel_spec.radius * dt)
                    .rem_euclid(std::f64::consts::TAU);
            }
        }

        // Mild slide recovery: resist yaw faster than the tyres could sustain.
        if self.assists.stability > 0.0 && forward_speed > 5.0 {
            let sustainable = total_grip / spec.mass / forward_speed;
            let excess = self.yaw_rate.abs() - sustainable * 1.05;
            if excess > 0.0 {
                yaw_moment -= self.yaw_rate.signum()
                    * excess
                    * spec.yaw_inertia
                    * 24.0
                    * self.assists.stability;
            }
        }

        // Drag, and gravity along the slope (uphill slows, downhill speeds up).
        if speed > 0.01 {
            force -= self.velocity / speed * drag;
        }
        let normal = surface_normal.normalize_or(DVec3::Y);
        force += DVec2::new(normal.x, normal.z) * normal.y * spec.mass * GRAVITY;

        let acceleration = force / spec.mass;
        self.velocity += acceleration * dt;
        self.yaw_rate += yaw_moment / spec.yaw_inertia * dt;
        if speed < 0.5 && throttle < 0.05 {
            self.velocity *= (1.0 - dt * 4.0).max(0.0);
            self.yaw_rate *= (1.0 - dt * 8.0).max(0.0);
        }
        self.yaw += self.yaw_rate * dt;
        let new_cg = cg + self.velocity * dt;
        let body = DVec2::new(acceleration.dot(forward), acceleration.dot(left));
        self.acceleration += (body - self.acceleration) * (dt * 12.0).min(1.0);

        // Ride height, attitude and vertical acceleration from the four contacts.
        let heights: [f64; 4] = std::array::from_fn(|index| self.wheels[index].ground_height);
        let ground = heights.iter().sum::<f64>() * 0.25;
        let vertical_velocity = (ground - self.position.y) / dt;
        let vertical_acceleration = (vertical_velocity - self.vertical_velocity) / dt;
        self.vertical_velocity = vertical_velocity;
        // Heavily filtered: compressions (Eau Rouge) load the car, crests unload it.
        self.vertical_acceleration += (vertical_acceleration.clamp(-30.0, 30.0)
            - self.vertical_acceleration)
            * (dt * 6.0).min(1.0);
        self.position = DVec3::new(new_cg.x, ground, new_cg.y);
        let front_height = (heights[0] + heights[1]) * 0.5;
        let rear_height = (heights[2] + heights[3]) * 0.5;
        let left_height = (heights[0] + heights[2]) * 0.5;
        let right_height = (heights[1] + heights[3]) * 0.5;
        let [height, pitch, roll] = &mut self.suspension;
        self.body_height = height.track(ground, dt);
        // Dive, squat and body roll go through the same suspension.
        self.pitch = pitch.track(
            ((front_height - rear_height) / wheelbase).atan() + self.acceleration.x * 0.0016,
            dt,
        );
        self.roll = roll.track(
            ((left_height - right_height) / ((front_track + rear_track) * 0.5)).atan()
                + self.acceleration.y * 0.0022,
            dt,
        );

        // Tyre temperature and wear per axle.
        for axle in 0..2 {
            self.tyres[axle].update(
                self.setup.compound,
                slide_power[axle],
                rolling_power[axle],
                speed,
                dt,
            );
        }
        self.track_position = track.project(self.position, Some(self.track_position.station));
    }

    /// Collision proxies: three circles along the body (front, middle, rear).
    pub fn collision_circles(&self) -> [(DVec2, f64); 3] {
        let radius = self.spec.width * 0.5;
        let reach = (self.spec.length * 0.5 - radius).max(0.0);
        let center =
            DVec2::new(self.position.x, self.position.z) + self.forward() * (-self.spec.cg_z());
        let forward = self.forward();
        [
            (center + forward * reach, radius),
            (center, radius),
            (center - forward * reach, radius),
        ]
    }

    /// Applies an impulse (N·s) at a ground-plane point.
    pub fn apply_impulse(&mut self, point: DVec2, impulse: DVec2) {
        let arm = point - DVec2::new(self.position.x, self.position.z);
        self.velocity += impulse / self.spec.mass;
        self.yaw_rate -= arm.perp_dot(impulse) / self.spec.yaw_inertia;
        self.impact = self.impact.max(impulse.length());
    }

    /// Moves the car without changing its velocity (penetration resolution).
    pub fn translate(&mut self, offset: DVec2) {
        self.position.x += offset.x;
        self.position.z += offset.y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::oval_track;
    use spec::test_wheels;

    fn car_on(track: &Track) -> Car {
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

    fn run(car: &mut Car, track: &Track, input: CarInput, seconds: f64) {
        let dt = 1.0 / 360.0;
        for _ in 0..(seconds / dt) as usize {
            car.step(&input, track, dt);
        }
    }

    /// Times from rest to each speed (km/h) at full throttle with automatic
    /// gears, plus the speed reached after `seconds`.
    fn launch(car: &mut Car, marks: &[f64], seconds: f64) -> (Vec<f64>, f64) {
        let track = oval_track();
        let input = CarInput {
            throttle: 1.0,
            ..CarInput::default()
        };
        let dt = 1.0 / 360.0;
        let mut times = vec![f64::NAN; marks.len()];
        for tick in 0..(seconds / dt) as usize {
            // Stay on the straight: put the car back on the grid every half
            // second, keeping its speed and gear.
            if tick % 180 == 0 {
                let (position, yaw) = track.grid_slot(0);
                let speed = car.speed();
                car.position = position;
                car.yaw = yaw;
                car.yaw_rate = 0.0;
                car.velocity = car.forward() * speed;
                car.track_position = track.project(position, None);
            }
            car.step(&input, &track, dt);
            for (mark, time) in marks.iter().zip(&mut times) {
                if time.is_nan() && car.speed() * 3.6 >= *mark {
                    *time = tick as f64 * dt;
                }
            }
        }
        (times, car.speed() * 3.6)
    }

    #[test]
    fn manual_seventh_reaches_its_shift_point_and_eighth_keeps_pulling() {
        let track = oval_track();
        for wing in [0.0, 5.0, 10.0] {
            let mut car = car_on(&track);
            car.assists.automatic_gears = false;
            car.setup.front_wing = wing;
            car.setup.rear_wing = wing;
            car.velocity = car.forward() * (260.0 / 3.6);
            car.powertrain.gear = 7;
            let mut shifted_at = None;
            for _ in 0..12 {
                let speed = car.speed();
                let (position, yaw) = track.grid_slot(0);
                car.position = position;
                car.yaw = yaw;
                car.yaw_rate = 0.0;
                car.velocity = car.forward() * speed;
                car.track_position = track.project(position, None);
                run(
                    &mut car,
                    &track,
                    CarInput {
                        throttle: 1.0,
                        ..CarInput::default()
                    },
                    0.5,
                );
                if car.powertrain.rpm >= car.spec.engine.rev_limit * 0.985 {
                    shifted_at = Some(car.speed());
                    run(
                        &mut car,
                        &track,
                        CarInput {
                            throttle: 1.0,
                            shift_up: true,
                            ..CarInput::default()
                        },
                        1.0 / 360.0,
                    );
                    break;
                }
            }
            let shift_speed = shifted_at.expect("seventh must reach the manual shift lights");
            assert_eq!(car.powertrain.gear, 8);
            launch(&mut car, &[], 5.0);
            eprintln!(
                "manual wing {wing}: shift at {:.1}, eighth {:.1} km/h",
                shift_speed * 3.6,
                car.speed() * 3.6
            );
            assert_eq!(car.powertrain.gear, 8);
            assert!(
                car.speed() > shift_speed + 0.5,
                "wing {wing}: eighth stopped accelerating"
            );
        }
    }

    #[test]
    fn gen22_reaches_eighth_without_drs_or_a_tow() {
        let track = oval_track();
        for wing in [0.0, 5.0, 10.0] {
            let mut car = car_on(&track);
            car.setup.front_wing = wing;
            car.setup.rear_wing = wing;
            let (times, speed) = launch(&mut car, &[280.0, 300.0], 30.0);
            eprintln!(
                "wing {wing}: gear {}, speed {speed:.1}, rpm {:.0}, 280/300 {times:?}",
                car.powertrain.gear, car.powertrain.rpm
            );
            assert_eq!(car.powertrain.gear, 8, "wing {wing}: {speed:.1} km/h");
        }
    }

    #[test]
    fn launch_and_top_speed_match_a_race_trim_f1_car() {
        let track = oval_track();
        let mut car = car_on(&track);
        let (times, top) = launch(&mut car, &[100.0, 200.0, 300.0], 30.0);
        // Roughly 2.6 s to 100 km/h, 5 s to 200 and 10-11 s to 300, then
        // ~320-330 km/h flat out without a tow or DRS.
        assert!((2.4..3.0).contains(&times[0]), "{times:?}");
        assert!((4.6..5.4).contains(&times[1]), "{times:?}");
        assert!((10.0..11.5).contains(&times[2]), "{times:?}");
        assert!((318.0..335.0).contains(&top), "{top} km/h");
    }

    #[test]
    fn automatic_gears_blip_down_under_braking() {
        let track = oval_track();
        let mut car = car_on(&track);
        launch(&mut car, &[], 9.0);
        assert!(car.powertrain.gear >= 7, "gear {}", car.powertrain.gear);
        let brake = CarInput {
            brake: 1.0,
            ..CarInput::default()
        };
        // Stay on the straight while braking from ~280 km/h.
        for _ in 0..12 {
            let (position, yaw) = track.grid_slot(0);
            let speed = car.speed();
            car.position = position;
            car.yaw = yaw;
            car.yaw_rate = 0.0;
            car.velocity = car.forward() * speed;
            run(&mut car, &track, brake, 0.25);
            let wheel_rpm = powertrain::Powertrain::wheel_rpm(
                &car.spec.engine,
                car.powertrain.gear,
                car.forward_speed(),
                car.spec.wheels[2].radius,
            );
            if car.speed() > 25.0 {
                // A gear low enough to keep the engine in its power band.
                assert!(
                    wheel_rpm > car.spec.engine.peak_power_rpm * 0.7,
                    "{:.0} km/h in gear {} at {wheel_rpm:.0} rpm",
                    car.speed() * 3.6,
                    car.powertrain.gear
                );
            }
        }
    }

    #[test]
    fn accelerates_in_a_straight_line() {
        let track = oval_track();
        let mut car = car_on(&track);
        let throttle = CarInput {
            throttle: 1.0,
            ..CarInput::default()
        };
        run(&mut car, &track, throttle, 2.7);
        // A modern F1 car reaches 100 km/h in roughly 2.6 seconds.
        let kmh = car.speed() * 3.6;
        assert!((90.0..115.0).contains(&kmh), "{kmh} km/h");
        run(&mut car, &track, throttle, 1.0);
        assert!(car.yaw_rate.abs() < 0.05, "yaw drift {}", car.yaw_rate);
        assert!(car.powertrain.gear >= 2);
    }

    #[test]
    fn brakes_hard_without_locking_under_assist() {
        let track = oval_track();
        let mut car = car_on(&track);
        car.velocity = car.forward() * 80.0;
        car.powertrain.gear = 7;
        run(
            &mut car,
            &track,
            CarInput {
                brake: 1.0,
                ..CarInput::default()
            },
            0.5,
        );
        let decel = -car.acceleration.x / GRAVITY;
        assert!(decel > 3.0, "{decel} g");
        assert!(car.wheels.iter().all(|wheel| !wheel.locked));
    }

    #[test]
    fn steering_left_turns_left() {
        let track = oval_track();
        let mut car = car_on(&track);
        car.velocity = car.forward() * 30.0;
        let start_yaw = car.yaw;
        run(
            &mut car,
            &track,
            CarInput {
                steer: 0.5,
                throttle: 0.3,
                ..CarInput::default()
            },
            0.5,
        );
        assert!(
            car.yaw > start_yaw + 0.1,
            "yaw {} -> {}",
            start_yaw,
            car.yaw
        );
    }

    #[test]
    fn slipstream_raises_top_speed() {
        let track = oval_track();
        let flat_out = CarInput {
            throttle: 1.0,
            ..CarInput::default()
        };
        let top_speed = |drag_scale: f64| {
            let mut car = car_on(&track);
            car.velocity = car.forward() * 60.0;
            car.powertrain.gear = 6;
            car.aero.drag_scale = drag_scale;
            // Stay on the straight by resetting position each second.
            for _ in 0..12 {
                let (position, yaw) = track.grid_slot(0);
                car.position = position;
                car.yaw = yaw;
                car.yaw_rate = 0.0;
                car.velocity = car.forward() * car.speed();
                run(&mut car, &track, flat_out, 1.0);
            }
            car.speed()
        };
        assert!(top_speed(0.85) > top_speed(1.0) + 1.0);
    }

    #[test]
    fn drs_opens_only_when_available_and_braking_closes_it() {
        let track = oval_track();
        let mut car = car_on(&track);
        let request = CarInput {
            throttle: 1.0,
            drs: true,
            ..CarInput::default()
        };
        run(&mut car, &track, request, 0.5);
        assert!(
            !car.drs.open && car.drs.flap == 0.0,
            "opened outside a zone"
        );
        car.drs.available = true;
        run(&mut car, &track, request, crate::drs::ACTUATION + 0.01);
        assert!(car.drs.open && car.drs.flap == 1.0);
        // The flap stays open without holding the button, until the brakes go on.
        let cruise = CarInput {
            throttle: 1.0,
            ..CarInput::default()
        };
        run(&mut car, &track, cruise, 0.1);
        assert!(car.drs.open);
        let brake = CarInput {
            brake: 0.5,
            ..CarInput::default()
        };
        run(&mut car, &track, brake, crate::drs::ACTUATION + 0.01);
        assert!(!car.drs.open && car.drs.flap == 0.0);
        // Re-opening needs a fresh request.
        run(&mut car, &track, cruise, 0.1);
        assert!(!car.drs.open);
    }

    #[test]
    fn automatic_drs_opens_when_available_and_closes_without_a_button() {
        let track = oval_track();
        let mut car = car_on(&track);
        car.assists.auto_drs = true;
        let cruise = CarInput {
            throttle: 1.0,
            ..CarInput::default()
        };
        let brake = CarInput {
            brake: 0.5,
            ..CarInput::default()
        };
        let settle = crate::drs::ACTUATION + 0.01;
        run(&mut car, &track, cruise, 0.3);
        assert_eq!(car.drs.flap, 0.0, "opened without DRS available");
        car.drs.available = true;
        run(&mut car, &track, cruise, settle);
        assert!(car.drs.open && car.drs.flap == 1.0);
        run(&mut car, &track, brake, settle);
        assert_eq!(car.drs.flap, 0.0, "braking must close it");
        // Off the brakes while still available: it opens again by itself.
        run(&mut car, &track, cruise, settle);
        assert_eq!(car.drs.flap, 1.0);
        // The end of the zone withdraws DRS and the flap closes.
        car.drs.available = false;
        run(&mut car, &track, cruise, settle);
        assert!(!car.drs.open && car.drs.flap == 0.0);
    }

    #[test]
    fn open_drs_raises_top_speed_and_costs_rear_downforce() {
        let track = oval_track();
        let flat_out = |drs| CarInput {
            throttle: 1.0,
            drs,
            ..CarInput::default()
        };
        let run_flat = |drs: bool| {
            let mut car = car_on(&track);
            car.velocity = car.forward() * 60.0;
            car.powertrain.gear = 6;
            car.drs.available = drs;
            for _ in 0..12 {
                let (position, yaw) = track.grid_slot(0);
                car.position = position;
                car.yaw = yaw;
                car.yaw_rate = 0.0;
                car.velocity = car.forward() * car.speed();
                run(&mut car, &track, flat_out(drs), 1.0);
            }
            (car.speed(), car.downforce / car.speed().powi(2))
        };
        let (closed, closed_downforce) = run_flat(false);
        let (open, open_downforce) = run_flat(true);
        assert!(open > closed + 1.5, "{closed} -> {open} m/s");
        assert!(open_downforce < closed_downforce * 0.95);
    }
}

#[cfg(test)]
mod debug_probe {
    use super::*;
    use crate::track::oval_track;
    use spec::test_wheels;

    #[test]
    #[ignore]
    fn probe_corner() {
        let track = oval_track();
        let mut car = Car::new(
            Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            &track,
            glam::DVec3::new(0.0, 0.0, 0.0),
            0.0,
        );
        car.velocity = car.forward() * 34.0;
        car.powertrain.gear = 4;
        let dt = 1.0 / 360.0;
        for tick in 0..(4.0 / dt) as usize {
            let throttle = if car.speed() < 34.0 { 0.6 } else { 0.0 };
            car.step(
                &CarInput {
                    steer: -1.0,
                    throttle,
                    ..CarInput::default()
                },
                &track,
                dt,
            );
            if tick < 120 && tick % 8 == 0 {
                let cgz = car.spec.cg_z();
                let m: f64 = (0..4)
                    .map(|i| (car.spec.wheels[i].position.z - cgz) * car.wheels[i].lateral_force)
                    .sum();
                println!(
                    "  t={:.3} r={:.3} fy={:.0},{:.0},{:.0},{:.0} Mz~{:.0} vy={:.2}",
                    tick as f64 * dt,
                    car.yaw_rate,
                    car.wheels[0].lateral_force,
                    car.wheels[1].lateral_force,
                    car.wheels[2].lateral_force,
                    car.wheels[3].lateral_force,
                    m,
                    car.velocity.dot(car.left())
                );
            }
            if tick % 90 == 0 {
                println!("t={:.2} v={:.1} yawrate={:.3} R={:.1} ay={:.1} steer={:.3} lim={:.3} slip={:.3},{:.3},{:.3},{:.3} load={:.0},{:.0},{:.0},{:.0} slide={:.2},{:.2}",
                    tick as f64 * dt, car.speed(), car.yaw_rate, car.speed()/car.yaw_rate.abs().max(1e-6), car.acceleration.y,
                    car.steer_angle, car.steer_limit(car.speed()),
                    car.wheels[0].slip_angle, car.wheels[1].slip_angle, car.wheels[2].slip_angle, car.wheels[3].slip_angle,
                    car.wheels[0].load, car.wheels[1].load, car.wheels[2].load, car.wheels[3].load,
                    car.wheels[0].sliding, car.wheels[2].sliding);
            }
        }
    }

    #[test]
    #[ignore]
    fn probe_launch() {
        let track = oval_track();
        let (position, yaw) = track.grid_slot(0);
        let mut car = Car::new(
            Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                ..Assists::default()
            },
            &track,
            position,
            yaw,
        );
        let dt = 1.0 / 360.0;
        for tick in 0..(3.0 / dt) as usize {
            car.step(
                &CarInput {
                    throttle: 1.0,
                    ..CarInput::default()
                },
                &track,
                dt,
            );
            if tick % 90 == 0 {
                println!("t={:.2} v={:.1} gear={} rpm={:.0} spin={:?} load={:.0},{:.0} ax={:.1} y={:.2} s={:.1} lat={:.2}",
                    tick as f64 * dt, car.speed()*3.6, car.powertrain.gear, car.powertrain.rpm,
                    car.wheels.iter().map(|w| w.spinning).collect::<Vec<_>>(), car.wheels[0].load, car.wheels[2].load,
                    car.acceleration.x, car.position.y, car.track_position.s, car.track_position.lateral);
            }
        }
    }
}

#[cfg(test)]
mod abs_stability {
    use super::*;
    use crate::track::oval_track;
    use spec::test_wheels;

    /// Heading change over 1.2 s with a touch of lock, braking or coasting.
    fn heading_change(kmh: f64, brake: f64) -> (f64, f64) {
        let track = oval_track();
        let (position, yaw) = track.grid_slot(0);
        let mut car = Car::new(
            Arc::new(CarSpec::gen22(test_wheels(), 5.4, 1.9)),
            Setup::default(),
            Assists {
                automatic_gears: true,
                braking: 1.0,
                ..Assists::default()
            },
            &track,
            position,
            yaw,
        );
        car.velocity = car.forward() * (kmh / 3.6);
        car.powertrain.gear = 7;
        let dt = 1.0 / 360.0;
        let start = car.yaw;
        for _ in 0..(1.2 / dt) as usize {
            car.step(
                &CarInput {
                    brake,
                    steer: 0.05,
                    ..CarInput::default()
                },
                &track,
                dt,
            );
        }
        ((car.yaw - start).to_degrees().abs(), car.speed())
    }

    #[test]
    fn abs_braking_with_a_little_lock_does_not_spin_the_car() {
        for kmh in [300.0, 150.0, 90.0] {
            let (coasting, coast_speed) = heading_change(kmh, 0.0);
            let (braking, brake_speed) = heading_change(kmh, 1.0);
            assert!(
                brake_speed < coast_speed - 5.,
                "ABS must decelerate at {kmh} km/h"
            );
            assert!(
                braking < coasting * 2.0 + 3.0,
                "{kmh} km/h: braking turned {braking:.1} deg, coasting {coasting:.1} deg"
            );
        }
    }
}

#[cfg(test)]
mod steering_stability {
    use super::*;
    use crate::track::oval_track;
    use spec::test_wheels;

    const DT: f64 = 1.0 / 360.0;

    /// Steering and stability aid levels: the player default and Beginner
    /// preset run the yaw ceiling; the supported AI profile retains slide
    /// recovery below the ceiling threshold.
    const FULL_AIDS: (f64, f64) = (1.0, 1.0);
    const AI_AIDS: (f64, f64) = (1.0, AI_STABILITY);

    fn gen22() -> CarSpec {
        CarSpec::gen22(test_wheels(), 5.4, 1.9)
    }

    fn rb6() -> CarSpec {
        CarSpec::rb6(test_wheels(), 5.1, 1.8)
    }

    /// A car at `kmh` on flat asphalt, kept on the straight every tick with
    /// its heading and speed, so only the steering and the tyres act on it.
    struct Skidpad {
        track: Track,
        position: DVec3,
        car: Car,
        kmh: f64,
    }

    impl Skidpad {
        fn new(kmh: f64) -> Self {
            Self::with(kmh, gen22(), AI_AIDS)
        }

        fn with(kmh: f64, spec: CarSpec, (steering, stability): (f64, f64)) -> Self {
            let track = oval_track();
            let (position, yaw) = track.grid_slot(0);
            let mut car = Car::new(
                Arc::new(spec),
                Setup::default(),
                Assists {
                    automatic_gears: true,
                    steering,
                    stability,
                    ..Assists::default()
                },
                &track,
                position,
                yaw,
            );
            car.velocity = car.forward() * (kmh / 3.6);
            car.powertrain.gear = 7;
            Self {
                track,
                position,
                car,
                kmh,
            }
        }

        fn step(&mut self, steer: f64) {
            self.car.position = self.position;
            self.car.velocity = self.car.velocity.normalize() * (self.kmh / 3.6);
            let input = CarInput {
                steer,
                throttle: 0.4,
                ..CarInput::default()
            };
            self.car.step(&input, &self.track, DT);
        }

        fn hold(&mut self, steer: f64, seconds: f64) {
            for _ in 0..(seconds / DT) as usize {
                self.step(steer);
            }
        }

        /// Drives `keys` (-1, 0 or 1 at each time) through the game's
        /// keyboard ramp, 3.2/s in and 7/s back or across, and returns the
        /// yaw rate at every tick.
        fn keyboard(&mut self, seconds: f64, keys: impl Fn(f64) -> f64) -> Vec<f64> {
            let mut steer: f64 = 0.0;
            (0..(seconds / DT) as usize)
                .map(|tick| {
                    let key = keys(tick as f64 * DT);
                    let rate = if key == 0.0 || key.signum() != steer.signum() {
                        7.0
                    } else {
                        3.2
                    };
                    steer += (key - steer).clamp(-rate * DT, rate * DT);
                    self.step(steer);
                    self.car.yaw_rate
                })
                .collect()
        }

        fn front_slip(&self) -> f64 {
            let wheels = &self.car.wheels;
            (wheels[0].slip_angle.abs() + wheels[1].slip_angle.abs()) * 0.5
                / self.car.spec.peak_slip_angle
        }
    }

    /// Yaw response to the keyboard held into a turn from straight running.
    #[derive(Debug)]
    struct TurnIn {
        /// Mean yaw rate 1.5–2 s in, rad/s.
        steady: f64,
        /// First peak over `steady`, and the lowest point after it under it.
        overshoot: f64,
        undershoot: f64,
        /// Last time the yaw rate is more than 5% from `steady`, s.
        settle: f64,
        lateral_acceleration: f64,
        front_slip: f64,
    }

    fn turn_in(kmh: f64, spec: CarSpec, aids: (f64, f64)) -> TurnIn {
        let mut pad = Skidpad::with(kmh, spec, aids);
        let yaw = pad.keyboard(2.0, |_| 1.0);
        let from = (1.5 / DT) as usize;
        let steady = yaw[from..].iter().sum::<f64>() / (yaw.len() - from) as f64;
        let (peak_at, peak) =
            yaw.iter().copied().enumerate().fold(
                (0, 0.0),
                |best, (i, r)| if r > best.1 { (i, r) } else { best },
            );
        let trough = yaw[peak_at..].iter().copied().fold(f64::MAX, f64::min);
        let settle = yaw
            .iter()
            .rposition(|r| (r - steady).abs() > 0.05 * steady)
            .map_or(0.0, |i| (i + 1) as f64 * DT);
        TurnIn {
            steady,
            overshoot: peak / steady - 1.0,
            undershoot: 1.0 - trough / steady,
            settle,
            lateral_acceleration: pad.car.acceleration.y,
            front_slip: pad.front_slip(),
        }
    }

    /// Keyboard taps left and right every 0.25 s for 1.5 s, then hands off:
    /// the largest yaw rate during the taps and how long the car takes to
    /// run straight (yaw rate under 0.05 rad/s for 0.1 s) after release.
    fn weave(kmh: f64, spec: CarSpec, aids: (f64, f64)) -> (f64, f64) {
        let mut pad = Skidpad::with(kmh, spec, aids);
        let yaw = pad.keyboard(3.0, |t| match t {
            t if t >= 1.5 => 0.0,
            t if ((t / 0.25) as usize).is_multiple_of(2) => 1.0,
            _ => -1.0,
        });
        let release = (1.6 / DT) as usize;
        let peak = yaw[..release].iter().fold(0.0_f64, |m, r| m.max(r.abs()));
        let window = (0.1 / DT) as usize;
        let straight = (release..yaw.len() - window)
            .find(|&i| yaw[i..i + window].iter().all(|r| r.abs() < 0.05))
            .map_or(f64::INFINITY, |i| (i - release) as f64 * DT);
        (peak, straight)
    }

    #[test]
    fn full_assisted_lock_keeps_the_fronts_near_their_peak_at_speed() {
        for kmh in [150.0, 250.0, 300.0] {
            let mut full = Skidpad::new(kmh);
            full.hold(1.0, 2.0);
            let best = [0.3, 0.4, 0.5, 0.6]
                .into_iter()
                .map(|steer| {
                    let mut pad = Skidpad::new(kmh);
                    pad.hold(steer, 2.0);
                    pad.car.acceleration.y
                })
                .fold(0.0, f64::max);
            let slip = full.front_slip();
            assert!(
                (1.0..1.4).contains(&slip),
                "{kmh} km/h: full lock runs the fronts at {slip:.2}x peak slip"
            );
            assert!(
                full.car.acceleration.y > best * 0.94,
                "{kmh} km/h: full lock corners at {:.1} m/s2, best {best:.1}",
                full.car.acceleration.y
            );
        }
    }

    #[test]
    fn opposite_lock_keeps_the_full_steering_limit() {
        for aids in [AI_AIDS, FULL_AIDS] {
            // The car yaws left throughout, as in a slide being caught.
            let wheel = |steer: f64| {
                let mut pad = Skidpad::with(250.0, gen22(), aids);
                for _ in 0..180 {
                    pad.car.yaw_rate = 0.5;
                    pad.step(steer);
                }
                (pad.car.steer_angle, pad.car.steer_limit(pad.car.speed()))
            };
            let (into, limit) = wheel(1.0);
            let (against, _) = wheel(-1.0);
            assert!(into < limit * 0.7, "turn-in lock {into:.3} of {limit:.3}");
            assert!(
                against < -limit * 0.98,
                "{aids:?}: opposite lock {against:.3} of {limit:.3}"
            );
        }
    }

    /// With both aids at full, held keyboard lock at speed brings the yaw
    /// rate onto the grip ceiling with no swing either side of it, holds the
    /// fronts at their peak and corners at least as hard as without the
    /// ceiling. Without it the yaw rate overshoots by 32–59% here.
    #[test]
    fn full_aids_turn_in_without_yaw_overshoot() {
        for (name, spec) in [("GEN22", gen22 as fn() -> CarSpec), ("RB6", rb6)] {
            for kmh in [150.0, 250.0, 300.0] {
                let aided = turn_in(kmh, spec(), FULL_AIDS);
                let plain = turn_in(kmh, spec(), AI_AIDS);
                let label = format!("{name} {kmh} km/h: {aided:?}");
                assert!(aided.overshoot < 0.08, "{label}");
                assert!(aided.undershoot < 0.05, "{label}");
                assert!(aided.settle < 0.45, "{label}");
                assert!((0.9..1.05).contains(&aided.front_slip), "{label}");
                assert!(plain.overshoot > 0.25, "{name} {kmh} km/h: {plain:?}");
                assert!(
                    aided.lateral_acceleration > plain.lateral_acceleration,
                    "{label}; without the ceiling {plain:?}"
                );
            }
        }
    }

    /// Left-right keyboard corrections at speed: with both aids at full the
    /// yaw rate stays within the steady ceiling, and the car runs straight
    /// as soon after release as without the aid.
    #[test]
    fn full_aids_keep_keyboard_weaving_within_the_ceiling() {
        for (name, spec) in [("GEN22", gen22 as fn() -> CarSpec), ("RB6", rb6)] {
            for kmh in [150.0, 250.0, 300.0] {
                let ceiling = turn_in(kmh, spec(), FULL_AIDS).steady;
                let (peak, straight) = weave(kmh, spec(), FULL_AIDS);
                let (plain_peak, plain_straight) = weave(kmh, spec(), AI_AIDS);
                assert!(
                    peak < ceiling * 1.06,
                    "{name} {kmh} km/h: weave yaw {peak:.3} rad/s, ceiling {ceiling:.3}"
                );
                assert!(plain_peak > ceiling * 1.2, "{name} {kmh}: {plain_peak:.3}");
                assert!(
                    straight <= plain_straight + DT,
                    "{name} {kmh} km/h: straight after {straight:.2} s, plain {plain_straight:.2} s"
                );
            }
        }
    }

    /// Braking hard with lock on at 250 km/h: the ceiling neither costs
    /// deceleration nor lets the yaw rate run away.
    #[test]
    fn yaw_ceiling_keeps_trail_braking_stable() {
        let brake_turn = |aids: (f64, f64)| {
            let mut pad = Skidpad::with(250.0, gen22(), aids);
            let mut peak_yaw: f64 = 0.0;
            for _ in 0..(1.5 / DT) as usize {
                pad.car.position = pad.position;
                let input = CarInput {
                    steer: 0.6,
                    brake: 1.0,
                    ..CarInput::default()
                };
                pad.car.step(&input, &pad.track, DT);
                peak_yaw = peak_yaw.max(pad.car.yaw_rate.abs());
            }
            let slip = pad.car.velocity.normalize().dot(pad.car.forward()).acos();
            (pad.car.speed(), peak_yaw, slip)
        };
        let (speed, peak_yaw, slip) = brake_turn(FULL_AIDS);
        let (plain_speed, plain_yaw, _) = brake_turn(AI_AIDS);
        assert!(
            speed < plain_speed + 0.5,
            "{speed:.1} m/s after braking, plain {plain_speed:.1}"
        );
        assert!(
            peak_yaw <= plain_yaw,
            "yaw {peak_yaw:.3}, plain {plain_yaw:.3}"
        );
        assert!(slip < 0.1, "body slip {slip:.3} rad");
    }

    /// The ceiling needs steering above half and stability at least half:
    /// Balanced and Beginner engage it, while unassisted driving does not.
    #[test]
    fn yaw_ceiling_is_off_below_stability_half() {
        for aids in [AI_AIDS, (0.6, 0.49), (1.0, 0.0), (0.0, 1.0), (0.0, 0.0)] {
            let mut pad = Skidpad::with(250.0, gen22(), aids);
            pad.hold(0.5, 0.3);
            let car = &pad.car;
            let lock = car.yaw_ceiling_lock(
                0.05,
                &[5000.0; 4],
                car.forward_speed(),
                car.velocity.dot(car.left()),
            );
            assert!(lock.is_none(), "{aids:?}");
        }
    }

    #[test]
    fn out_of_range_steering_assist_cannot_invert_turn_in_limit() {
        let mut pad = Skidpad::with(250.0, gen22(), (2.0, AI_STABILITY));
        assert!(pad.car.turn_in_limit(pad.car.speed()) >= 0.0);
        pad.step(1.0);
    }

    /// Keyboard turn-in and weaving at 150–300 km/h for both cars, with the
    /// AI's slide-recovery aids (no yaw ceiling) and with both aids at full.
    /// `cargo test -p racecraft-sim --lib probe_keyboard_turn_in -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_keyboard_turn_in() {
        for (name, spec) in [("GEN22", gen22 as fn() -> CarSpec), ("RB6", rb6)] {
            for (label, aids) in [("AI aids", AI_AIDS), ("full aids", FULL_AIDS)] {
                for kmh in [150.0, 250.0, 300.0] {
                    let r = turn_in(kmh, spec(), aids);
                    let (weave_peak, straight) = weave(kmh, spec(), aids);
                    println!(
                        "{name:>5} {label:>14} {kmh:>3} km/h: yaw {:.3} rad/s, overshoot {:>4.1}%, \
                         undershoot {:>4.1}%, settled {:.2} s, ay {:.1} m/s2, front {:.2}x peak; \
                         weave peak {weave_peak:.3} rad/s, straight {straight:.2} s after release",
                        r.steady,
                        r.overshoot * 100.0,
                        r.undershoot * 100.0,
                        r.settle,
                        r.lateral_acceleration,
                        r.front_slip,
                    );
                }
            }
        }
    }
}

/// Complete mutable vehicle state; immutable geometry/specification is never sent.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CarState {
    // Configuration, not dynamics: kept exact so it matches the room rules.
    #[serde(with = "crate::exact")]
    setup: Setup,
    #[serde(with = "crate::exact")]
    assists: Assists,
    position: DVec3,
    yaw: f64,
    yaw_rate: f64,
    velocity: DVec2,
    pitch: f64,
    roll: f64,
    body_height: f64,
    suspension: [Suspension; 3],
    steer_angle: f64,
    powertrain: Powertrain,
    tyres: [AxleTyres; 2],
    wheels: [WheelState; 4],
    aero: AeroInfluence,
    drs: Drs,
    downforce: f64,
    track_position: TrackPosition,
    acceleration: DVec2,
    vertical_velocity: f64,
    vertical_acceleration: f64,
    impact: f64,
}
impl Car {
    pub fn snapshot(&self) -> CarState {
        CarState {
            setup: self.setup,
            assists: self.assists,
            position: self.position,
            yaw: self.yaw,
            yaw_rate: self.yaw_rate,
            velocity: self.velocity,
            pitch: self.pitch,
            roll: self.roll,
            body_height: self.body_height,
            suspension: self.suspension,
            steer_angle: self.steer_angle,
            powertrain: self.powertrain,
            tyres: self.tyres,
            wheels: self.wheels,
            aero: self.aero,
            drs: self.drs,
            downforce: self.downforce,
            track_position: self.track_position,
            acceleration: self.acceleration,
            vertical_velocity: self.vertical_velocity,
            vertical_acceleration: self.vertical_acceleration,
            impact: self.impact,
        }
    }
    pub fn restore(&mut self, state: &CarState) {
        self.setup = state.setup;
        self.assists = state.assists;
        self.position = state.position;
        self.yaw = state.yaw;
        self.yaw_rate = state.yaw_rate;
        self.velocity = state.velocity;
        self.pitch = state.pitch;
        self.roll = state.roll;
        self.body_height = state.body_height;
        self.suspension = state.suspension;
        self.steer_angle = state.steer_angle;
        self.powertrain = state.powertrain;
        self.tyres = state.tyres;
        self.wheels = state.wheels;
        self.aero = state.aero;
        self.drs = state.drs;
        self.downforce = state.downforce;
        self.track_position = state.track_position;
        self.acceleration = state.acceleration;
        self.vertical_velocity = state.vertical_velocity;
        self.vertical_acceleration = state.vertical_acceleration;
        self.impact = state.impact;
    }
}
