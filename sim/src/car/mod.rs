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
use tyre::{lateral_shape, load_sensitivity, AxleTyres};

pub const AIR_DENSITY: f64 = 1.225;
pub const GRAVITY: f64 = 9.81;

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
    /// Mild slide recovery, 0..1.
    pub stability: f64,
    pub automatic_gears: bool,
    /// Lets this driver open DRS anywhere, not only where it is earned.
    #[serde(default)]
    pub drs_anywhere: bool,
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
        }
    }
}

/// The rear-wing flap. The race sets `available`; the driver opens it, and
/// braking or leaving the zone closes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Drs {
    pub available: bool,
    pub open: bool,
    /// Flap position, 0 closed to 1 fully open.
    pub flap: f64,
}

impl Drs {
    fn update(&mut self, request: bool, brake: f64, dt: f64) {
        if !self.available || brake > 0.05 {
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
#[derive(Clone, Copy, Debug, PartialEq)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
        const FULL_LOCK: f64 = 0.36;
        let spec = &self.spec;
        let lift = self.setup.front_lift_area(&spec.aero) + self.setup.rear_lift_area(&spec.aero);
        let lateral_grip =
            spec.tyre_friction * (GRAVITY + 0.5 * AIR_DENSITY * lift * speed * speed / spec.mass);
        let kinematic = (spec.wheelbase() * lateral_grip / speed.max(1.0).powi(2)).atan();
        // Lets the driver take the front tyres just past their peak: enough to
        // feel understeer when overdriving, not enough to make full lock correct.
        let assisted = (kinematic + spec.peak_slip_angle * 1.15).min(FULL_LOCK);
        FULL_LOCK + (assisted - FULL_LOCK) * self.assists.steering
    }

    pub fn step(&mut self, input: &CarInput, track: &Track, dt: f64) {
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
            self.powertrain
                .auto_shift(&spec.engine, forward_speed, rear_radius, input.throttle);
        }
        let throttle = input.throttle.clamp(0.0, 1.0);
        let brake = input.brake.clamp(0.0, 1.0);
        let drive =
            self.powertrain
                .drive_force(&spec.engine, throttle, forward_speed, rear_radius, dt);

        // Steering: smoothed toward a speed-limited target.
        let target = input.steer.clamp(-1.0, 1.0) * self.steer_limit(speed);
        let response = 30.0 - 18.0 * self.assists.steering;
        self.steer_angle += (target - self.steer_angle) * (dt * response).min(1.0);

        // Aerodynamics. An open DRS flap stalls part of the rear wing.
        self.drs.update(input.drs, brake, dt);
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

        for (index, wheel_spec) in spec.wheels.iter().enumerate() {
            let front = index < 2;
            let axle = if front { 0 } else { 1 };
            let lx = wheel_spec.position.z - cg_z;
            let ly = wheel_spec.position.x;
            let contact = &contacts[index];
            let (available, room) = (contact.available, contact.longitudinal_room);
            let wheel_long = contact.wheel_long;

            // Longitudinal demand: drive on the rear, brakes everywhere.
            let brake_share = if front {
                spec.brake_bias
            } else {
                1.0 - spec.brake_bias
            } * 0.5;
            let mut brake_force = brake_force_total * brake_share;
            let mut drive_force = if front { 0.0 } else { drive * 0.5 };

            // Partial ABS trims a locking demand to slightly under the limit, so
            // it never beats a driver who brakes right at the limit.
            let mut locked = false;
            if brake_force > room {
                if brake_force > room * (1.0 + 0.5 * self.assists.braking) + 50.0 {
                    locked = wheel_long.abs() > 0.5;
                } else {
                    brake_force = room * (1.0 - 0.06 * self.assists.braking);
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
