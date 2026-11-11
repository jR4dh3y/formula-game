//! Per-car driver, TV-pod and chase views, with live editable framing.
use crate::{pose::Pose, Session};
use bevy::{anti_alias::taa::TemporalAntiAliasing, prelude::*};
use glam::DVec3;
use racecraft_sim::Race;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Cockpit,
    Tv,
    Chase,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Cockpit, Self::Tv, Self::Chase];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cockpit => "Cockpit",
            Self::Tv => "TV pod",
            Self::Chase => "Third person",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub height: f64,
    pub fore_aft: f64,
    pub pitch: f64,
    pub fov: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tuning {
    pub cockpit: View,
    pub tv: View,
    pub chase: View,
    pub shake: f64,
    pub horizon: f64,
    #[serde(default = "default_dynamics")]
    pub dynamics: f64,
}

fn default_dynamics() -> f64 {
    0.65
}

impl Tuning {
    pub fn for_car(car: &str) -> Self {
        let modern = car.eq_ignore_ascii_case("gen22");
        Self {
            cockpit: View {
                height: if modern { 1.00 } else { 0.96 },
                fore_aft: -0.18,
                pitch: 3.,
                fov: 64.,
            },
            tv: View {
                height: if modern { 1.35 } else { 1.30 },
                fore_aft: if modern { -0.25 } else { -0.35 },
                pitch: 8.,
                fov: 60.,
            },
            chase: View {
                height: if modern { 1.85 } else { 1.65 },
                fore_aft: if modern { -6.2 } else { -5.6 },
                pitch: 4.5,
                fov: 54.,
            },
            shake: 0.25,
            horizon: 0.65,
            dynamics: default_dynamics(),
        }
    }
    pub fn view(&self, mode: Mode) -> View {
        match mode {
            Mode::Cockpit => self.cockpit,
            Mode::Tv => self.tv,
            Mode::Chase => self.chase,
        }
    }
    pub fn view_mut(&mut self, mode: Mode) -> &mut View {
        match mode {
            Mode::Cockpit => &mut self.cockpit,
            Mode::Tv => &mut self.tv,
            Mode::Chase => &mut self.chase,
        }
    }
    pub fn sanitize(&mut self) {
        let fallback = Self::for_car("rb6");
        for mode in Mode::ALL {
            let base = fallback.view(mode);
            let view = self.view_mut(mode);
            if !view.height.is_finite() {
                view.height = base.height;
            }
            if !view.fore_aft.is_finite() {
                view.fore_aft = base.fore_aft;
            }
            if !view.pitch.is_finite() {
                view.pitch = base.pitch;
            }
            if !view.fov.is_finite() {
                view.fov = base.fov;
            }
            view.height = view.height.clamp(0.65, 4.);
            view.fore_aft = if mode == Mode::Chase {
                view.fore_aft.clamp(-10., -3.5)
            } else {
                view.fore_aft.clamp(-1.5, 0.7)
            };
            view.pitch = view.pitch.clamp(-10., 20.);
            view.fov = view.fov.clamp(40., 95.);
        }
        self.dynamics = if self.dynamics.is_finite() {
            self.dynamics.clamp(0., 1.)
        } else {
            default_dynamics()
        };
        self.shake = if self.shake.is_finite() {
            self.shake.clamp(0., 1.)
        } else {
            0.25
        };
        self.horizon = if self.horizon.is_finite() {
            self.horizon.clamp(0., 1.)
        } else {
            0.65
        };
    }
}

pub struct ChaseCamera {
    pub mode: Mode,
    pub tuning: Tuning,
    position: DVec3,
    target: DVec3,
    yaw: f64,
    fov: f32,
    cut: bool,
    load: glam::DVec2,
    speed: f64,
    /// Share of the boom kept clear of a wall, and metres lifted over ground.
    boom: f64,
    lift: f64,
    cockpit_eye: Option<DVec3>,
    /// The player's held collision impulse (N·s), for the jolt.
    pub impact: f64,
    pub motion_enabled: bool,
}

impl ChaseCamera {
    pub fn new(race: &Race, player: usize) -> Self {
        let car = &race.entrants[player].car;
        let mut camera = Self {
            mode: Mode::Chase,
            tuning: Tuning::for_car(&car.spec.name),
            position: car.position,
            target: car.position,
            yaw: car.yaw,
            fov: 54.,
            cut: true,
            load: glam::DVec2::ZERO,
            speed: 0.,
            boom: 1.,
            lift: 0.,
            cockpit_eye: None,
            impact: 0.,
            motion_enabled: true,
        };
        camera.snap(&Pose::of(car));
        camera
    }
    pub fn set_cockpit_eye(&mut self, eye: Option<[f64; 3]>) {
        self.cockpit_eye = eye.map(DVec3::from_array);
        self.cut = true;
    }
    pub fn cycle(&mut self) {
        self.select(match self.mode {
            Mode::Chase => Mode::Cockpit,
            Mode::Cockpit => Mode::Tv,
            Mode::Tv => Mode::Chase,
        });
    }
    pub fn select(&mut self, mode: Mode) {
        if self.mode != mode {
            self.mode = mode;
            self.cut = true;
        }
    }
    pub fn configure(&mut self, mut tuning: Tuning) {
        tuning.sanitize();
        if self.tuning != tuning {
            self.tuning = tuning;
            self.cut = true;
        }
    }
    fn desired(&self, car: &Pose) -> (DVec3, DVec3) {
        let origin = car.origin;
        let view = self.tuning.view(self.mode);
        let pitch = view.pitch.to_radians();
        let direction = DVec3::new(0., -pitch.sin(), pitch.cos());
        if self.mode == Mode::Chase {
            let forward = DVec3::new(self.yaw.sin(), 0., self.yaw.cos());
            let strength = if self.motion_enabled {
                self.tuning.dynamics
            } else {
                0.
            };
            // The rig only breathes along the car's axis: back with speed and
            // under power, closer under braking. Sideways motion comes from the
            // yaw lag alone, so the car turns in the frame and the horizon stays put.
            let eye = origin
                + forward
                    * (view.fore_aft
                        - self.speed * strength * 0.6
                        - self.load.x * strength * 0.012)
                + DVec3::Y * view.height;
            (
                eye,
                eye + forward * (30. * pitch.cos()) - DVec3::Y * (30. * pitch.sin()),
            )
        } else {
            let attitude =
                glam::DQuat::from_euler(glam::EulerRot::YXZ, car.yaw, -car.pitch, car.roll);
            let stabilized = glam::DQuat::from_euler(
                glam::EulerRot::YXZ,
                car.yaw,
                -car.pitch * (1. - self.tuning.horizon),
                car.roll * (1. - self.tuning.horizon),
            );
            let offset = if self.mode == Mode::Cockpit {
                self.cockpit_eye
                    .map_or(DVec3::ZERO, |eye| eye - DVec3::new(0., 1., -0.18))
            } else {
                DVec3::ZERO
            };
            let eye = origin + attitude * (DVec3::new(0., view.height, view.fore_aft) + offset);
            (eye, eye + stabilized * direction * 30.)
        }
    }
    /// Eye position and look-at point; the audio listener follows them.
    pub fn listener(&self) -> (DVec3, DVec3) {
        (self.position, self.target)
    }
    fn snap(&mut self, pose: &Pose) {
        (self.position, self.target) = self.desired(pose);
    }
    pub fn update(
        &mut self,
        transform: &mut Transform,
        projection: &mut Projection,
        race: &Race,
        player: usize,
        pose: &Pose,
        dt: f32,
    ) -> bool {
        let dt = dt as f64;
        let car = &race.entrants[player].car;
        let cut = self.cut || self.position.distance(pose.origin) > 25.;
        let response = if cut { 1. } else { 1. - (-dt * 2.5).exp() };
        let load = glam::DVec2::new(car.acceleration.x.clamp(-45., 15.), 0.);
        self.load += (load - self.load) * response;
        self.speed += ((car.speed() / 90.).clamp(0., 1.) - self.speed) * response;
        if cut {
            self.boom = 1.;
            self.lift = 0.;
        }
        if cut {
            self.yaw = pose.yaw;
            self.snap(pose);
            self.cut = false;
        }
        let wrap = |angle: f64| {
            (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        };
        // At speed the chase view leans toward where the car is going rather
        // than where it points, so yaw wobble and slides turn the car in the
        // frame instead of swinging the whole world.
        let course = car.velocity.x.atan2(car.velocity.y);
        let lean = if self.mode == Mode::Chase && !cut {
            0.6 * ((car.speed() - 10.) / 20.).clamp(0., 1.)
        } else {
            0.
        };
        let aim = pose.yaw + wrap(course - pose.yaw).clamp(-0.6, 0.6) * lean;
        let delta = wrap(aim - self.yaw);
        self.yaw += delta * if cut { 1. } else { 1. - (-dt * 7.).exp() };
        let (position, target) = self.desired(pose);
        if self.mode != Mode::Chase {
            self.position = position;
            self.target = target;
        } else {
            if cut {
                self.position = position;
                self.target = target;
            } else {
                // Predict the translation before smoothing the load-dependent framing.
                let advance = DVec3::new(car.velocity.x, 0., car.velocity.y) * dt;
                self.position += advance;
                self.target += advance;
                let follow = 1. - (-dt * 12.).exp();
                self.position += (position - self.position) * follow;
                self.target += (target - self.target) * follow;
            }
        }
        let mut eye = self.position;
        if self.mode == Mode::Chase {
            // Ease the boom in where it would cross a barrier, and ride over
            // rising ground. Both are smoothed: neither cuts nor lifts the view.
            let anchor = glam::DVec2::new(pose.origin.x, pose.origin.z);
            let boom = glam::DVec2::new(self.position.x, self.position.z) - anchor;
            let mut clear = 1_f64;
            for wall in race.track.collision.walls_near(anchor, boom.length() + 1.) {
                let edge = wall.b - wall.a;
                let denominator = boom.perp_dot(edge);
                if denominator.abs() < 1e-9 {
                    continue;
                }
                let offset = wall.a - anchor;
                let t = offset.perp_dot(edge) / denominator;
                let u = offset.perp_dot(boom) / denominator;
                if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&u) {
                    clear = clear.min((t - 0.4 / boom.length().max(0.4)).max(0.55));
                }
            }
            self.boom += (clear - self.boom) * (1. - (-dt * 8.).exp());
            eye.x = anchor.x + boom.x * self.boom;
            eye.z = anchor.y + boom.y * self.boom;
            let needed = race
                .track
                .collision
                .ground(eye + DVec3::Y * 3.)
                .map_or(0., |ground| (ground.height + 0.6 - eye.y).max(0.));
            self.lift = needed.max(self.lift * (-dt * 4.).exp());
            eye.y += self.lift;
        }
        let kerb = car
            .wheels
            .iter()
            .map(|wheel| wheel.surface.roughness())
            .fold(0., f64::max);
        let vibration = ((race.time * 25.8).sin() + (race.time * 47.7).sin() * 0.35)
            * kerb
            * (car.speed() / 50.).min(1.)
            * 0.12;
        let shake = (vibration + (self.impact / 8000.).min(0.12)) * self.tuning.shake;
        *transform = Transform::from_translation(to_vec3(eye + DVec3::Y * shake))
            .looking_at(to_vec3(self.target), Vec3::Y);
        if let Projection::Perspective(lens) = projection {
            lens.near = if self.mode == Mode::Cockpit {
                // GEN22 needs close forearms; RB6 relies on clipping its double-sided tub.
                if self.cockpit_eye.is_some() {
                    0.04
                } else {
                    0.17
                }
            } else {
                0.1
            };
            let target = self.tuning.view(self.mode).fov
                + if self.mode == Mode::Chase {
                    self.speed as f32
                        * (3.
                            + if self.motion_enabled {
                                self.tuning.dynamics as f32 * 6.
                            } else {
                                0.
                            })
                } else {
                    0.
                };
            if cut {
                self.fov = target;
            } else {
                self.fov += (target - self.fov) * (1. - (-dt as f32 * 4.).exp());
            }
            lens.fov = self.fov.to_radians();
        }
        cut
    }
}

fn to_vec3(value: DVec3) -> Vec3 {
    Vec3::new(value.x as f32, value.y as f32, value.z as f32)
}

type CameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Projection,
        Option<&'static mut TemporalAntiAliasing>,
    ),
    (With<Camera3d>, With<bevy_egui::PrimaryEguiContext>),
>;

pub fn update(
    mut session: ResMut<Session>,
    mut cameras: CameraQuery,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    frontend: Res<crate::menu::Frontend>,
) {
    let dt = if session.paused {
        0.
    } else {
        time.delta_secs()
    };
    let Session {
        race,
        player,
        camera: rig,
        poses,
        impact,
        ..
    } = &mut *session;
    let pose = poses.at(race, *player, fixed.overstep_fraction_f64());
    for (mut transform, mut lens, taa) in &mut cameras {
        let previous = transform.translation;
        rig.impact = *impact;
        rig.motion_enabled = frontend.video.camera_motion;
        let cut = rig.update(&mut transform, &mut lens, race, *player, &pose, dt);
        if let Some(mut taa) = taa {
            taa.reset |= cut || transform.translation.distance(previous) > 15.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_camera_is_frame_rate_stable_and_teleports_cut_cleanly() {
        let mut offsets = Vec::new();
        for hz in [30, 60, 144, 240] {
            let mut session = crate::tests::session();
            let mut camera = ChaseCamera::new(&session.race, 0);
            let mut transform = Transform::default();
            let mut projection = Projection::Perspective(PerspectiveProjection::default());
            for frame in 0..=hz * 2 {
                let car = &mut session.race.entrants[0].car;
                car.position = DVec3::new(0., 0., frame as f64 / hz as f64 * 88.89);
                car.velocity = glam::DVec2::new(0., 88.89);
                let pose = Pose::of(car);
                camera.update(
                    &mut transform,
                    &mut projection,
                    &session.race,
                    0,
                    &pose,
                    1. / hz as f32,
                );
            }
            offsets.push(transform.translation.z as f64 - session.race.entrants[0].car.position.z);
            session.race.entrants[0].car.position.z += 100.;
            let pose = Pose::of(&session.race.entrants[0].car);
            assert!(camera.update(
                &mut transform,
                &mut projection,
                &session.race,
                0,
                &pose,
                1. / hz as f32
            ));
            let expected = camera.desired(&pose).0;
            assert!(transform.translation.as_dvec3().distance(expected) < 0.001);
        }
        let min = offsets.iter().copied().fold(f64::INFINITY, f64::min);
        let max = offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max - min < 0.05,
            "steady-speed framing differs: {offsets:?}"
        );
    }

    #[test]
    fn old_tuning_loads_and_dynamics_are_clamped() {
        let original = Tuning::for_car("gen22");
        let mut json = serde_json::to_value(&original).unwrap();
        json.as_object_mut().unwrap().remove("dynamics");
        let mut loaded: Tuning = serde_json::from_value(json).unwrap();
        assert_eq!(loaded, original);
        loaded.dynamics = f64::NAN;
        loaded.sanitize();
        assert_eq!(loaded.dynamics, default_dynamics());
    }

    #[test]
    fn paused_camera_preview_updates_without_drifting_with_car_velocity() {
        let mut session = crate::tests::session();
        session.race.entrants[0].car.velocity = glam::DVec2::new(40., 20.);
        let mut transform = Transform::default();
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        let pose = Pose::of(&session.race.entrants[0].car);
        let rig = &mut session.camera;
        rig.update(&mut transform, &mut projection, &session.race, 0, &pose, 0.);
        let initial = transform.translation;
        for _ in 0..100 {
            rig.update(&mut transform, &mut projection, &session.race, 0, &pose, 0.);
        }
        assert_eq!(transform.translation, initial);
        let mut tuning = rig.tuning.clone();
        tuning.chase.height += 0.5;
        rig.configure(tuning);
        assert!(rig.update(&mut transform, &mut projection, &session.race, 0, &pose, 0.));
        assert!((transform.translation.y - initial.y - 0.5).abs() < 0.001);
    }
}
