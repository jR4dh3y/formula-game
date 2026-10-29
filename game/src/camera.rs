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
}

impl Tuning {
    pub fn for_car(car: &str) -> Self {
        Self {
            cockpit: View {
                height: if car.eq_ignore_ascii_case("rb22") {
                    1.00
                } else {
                    0.96
                },
                fore_aft: -0.18,
                pitch: 3.,
                fov: 64.,
            },
            tv: View {
                height: 1.30,
                fore_aft: -0.35,
                pitch: 8.,
                fov: 60.,
            },
            chase: View {
                height: 1.65,
                fore_aft: -5.6,
                pitch: 4.5,
                fov: 54.,
            },
            shake: 0.25,
            horizon: 0.65,
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
        };
        camera.snap(&Pose::of(car));
        camera
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
            let eye = origin + forward * view.fore_aft + DVec3::Y * view.height;
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
            let eye = origin + attitude * DVec3::new(0., view.height, view.fore_aft);
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
        let cut = self.cut;
        if cut {
            self.yaw = pose.yaw;
            self.snap(pose);
            self.cut = false;
        }
        let delta = (pose.yaw - self.yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        self.yaw += delta * (1. - (-dt * 8.).exp());
        let (position, target) = self.desired(pose);
        if self.mode != Mode::Chase {
            self.position = position;
            self.target = target;
        } else {
            // Follow steady velocity without the several-metre lag of a plain lerp.
            let advance = DVec3::new(car.velocity.x, 0., car.velocity.y) * dt;
            self.position += advance;
            self.target += advance;
            let follow = 1. - (-dt * 12.).exp();
            self.position += (position - self.position) * follow;
            self.target += (target - self.target) * follow;
            if let Some(ground) = race.track.collision.ground(self.position + DVec3::Y * 3.) {
                self.position.y = self.position.y.max(ground.height + 0.6);
            }
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
        let shake = (vibration + (car.impact / 8000.).min(0.12)) * self.tuning.shake;
        *transform = Transform::from_translation(to_vec3(self.position + DVec3::Y * shake))
            .looking_at(to_vec3(self.target), Vec3::Y);
        if let Projection::Perspective(lens) = projection {
            lens.near = if self.mode == Mode::Cockpit {
                0.17
            } else {
                0.1
            };
            let target = self.tuning.view(self.mode).fov
                + if self.mode == Mode::Chase {
                    (car.speed() as f32 / 90.).min(1.) * 3.
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
    With<Camera3d>,
>;

pub fn update(
    mut session: ResMut<Session>,
    mut cameras: CameraQuery,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
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
        ..
    } = &mut *session;
    let pose = poses.at(race, *player, fixed.overstep_fraction_f64());
    for (mut transform, mut lens, taa) in &mut cameras {
        let previous = transform.translation;
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
