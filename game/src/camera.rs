//! Chase, T-cam and trackside cameras.

use dhool_pro::math::Vec3;
use dhool_pro::World;
use glam::DVec3;
use racecraft_sim::Race;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Chase,
    Far,
    TCam,
    Nose,
}

pub struct ChaseCamera {
    pub mode: Mode,
    position: DVec3,
    target: DVec3,
    yaw: f64,
}

impl ChaseCamera {
    pub fn new(race: &Race, player: usize) -> Self {
        let car = &race.entrants[player].car;
        let mut camera = Self {
            mode: Mode::Chase,
            position: car.position,
            target: car.position,
            yaw: car.yaw,
        };
        camera.snap(race, player);
        camera
    }

    pub fn cycle(&mut self) {
        self.mode = match self.mode {
            Mode::Chase => Mode::Far,
            Mode::Far => Mode::TCam,
            Mode::TCam => Mode::Nose,
            Mode::Nose => Mode::Chase,
        };
    }

    fn desired(&self, race: &Race, player: usize) -> (DVec3, DVec3) {
        let car = &race.entrants[player].car;
        let (sin, cos) = self.yaw.sin_cos();
        let forward = DVec3::new(sin, 0.0, cos);
        let origin = car.model_origin();
        match self.mode {
            Mode::Chase => (
                origin - forward * 6.2 + DVec3::Y * 2.0,
                origin + forward * 6.0 + DVec3::Y * 0.9,
            ),
            Mode::Far => (
                origin - forward * 10.5 + DVec3::Y * 3.4,
                origin + forward * 8.0 + DVec3::Y * 0.8,
            ),
            Mode::TCam => {
                let eye = car.model_to_world(DVec3::new(0.0, 1.32, -0.35));
                (eye, eye + forward * 20.0 - DVec3::Y * 1.6)
            }
            Mode::Nose => {
                let eye = car.model_to_world(DVec3::new(0.0, 0.62, 1.2));
                (eye, eye + forward * 20.0 - DVec3::Y * 0.6)
            }
        }
    }

    fn snap(&mut self, race: &Race, player: usize) {
        let (position, target) = self.desired(race, player);
        self.position = position;
        self.target = target;
    }

    pub fn update(&mut self, world: &mut World, race: &Race, player: usize, dt: f32) {
        let dt = dt as f64;
        let car = &race.entrants[player].car;
        let onboard = matches!(self.mode, Mode::TCam | Mode::Nose);
        // The chase camera follows the car's heading with a little lag so slides read.
        let yaw_lag = if onboard { 40.0 } else { 7.0 };
        let delta = (car.yaw - self.yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        self.yaw += delta * (dt * yaw_lag).min(1.0);
        let (position, target) = self.desired(race, player);
        if onboard {
            self.position = position;
            self.target = target;
        } else {
            let follow = (dt * 14.0).min(1.0);
            self.position += (position - self.position) * follow;
            self.target += (target - self.target) * follow;
            // Keep the camera above the track surface over crests and compressions.
            if let Some(ground) = race.track.collision.ground(self.position + DVec3::Y * 3.0) {
                self.position.y = self.position.y.max(ground.height + 0.8);
            }
        }
        let shake = (car.impact / 8_000.0).min(0.25);
        let kerb = car
            .wheels
            .iter()
            .map(|wheel| wheel.surface.roughness())
            .fold(0.0, f64::max);
        let jitter =
            (shake + kerb * 2.0 * (car.speed() / 40.0).min(1.0)) * (race.time * 57.0).sin();
        if let Some((_, rig)) = world.active_camera_mut() {
            rig.camera.position = to_vec3(self.position + DVec3::Y * jitter * 0.3);
            rig.camera.target = to_vec3(self.target);
            rig.camera.vertical_fov_radians = match self.mode {
                Mode::TCam | Mode::Nose => 70.0_f32.to_radians(),
                _ => (60.0 + (car.speed() as f32 / 90.0).min(1.0) * 6.0).to_radians(),
            };
        }
    }
}

fn to_vec3(value: DVec3) -> Vec3 {
    Vec3::new(value.x as f32, value.y as f32, value.z as f32)
}
