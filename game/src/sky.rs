//! Photographic dry-weather skies and matched scene lighting.

use crate::{graphics::Level, menu::Frontend, Session};
use bevy::{
    asset::RenderAssetUsages,
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    image::{CompressedImageFormats, ImageSampler, ImageType},
    light::{
        CascadeShadowConfigBuilder, DirectionalLightShadowMap, GeneratedEnvironmentMapLight,
        ShadowFilteringMethod, Skybox,
    },
    prelude::*,
    render::{
        camera::MipBias,
        render_resource::{TextureViewDescriptor, TextureViewDimension},
    },
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weather {
    #[default]
    Clear,
    Storm,
}

impl Weather {
    pub const ALL: [Self; 2] = [Self::Clear, Self::Storm];

    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear sky",
            Self::Storm => "Storm",
        }
    }

    pub fn cli(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Storm => "storm",
        }
    }

    fn brightness(self) -> f32 {
        match self {
            Self::Clear => 12_000.,
            Self::Storm => 1_800.,
        }
    }
}

impl std::str::FromStr for Weather {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "clear" => Ok(Self::Clear),
            "storm" => Ok(Self::Storm),
            _ => Err("weather must be clear or storm".into()),
        }
    }
}

#[derive(Component)]
pub(crate) struct Sun;

#[derive(Resource)]
pub(crate) struct Skies([[Handle<Image>; 3]; 2]);

/// Visual lightning and delayed thunder share the same paused race clock.
#[derive(Resource)]
pub(crate) struct Storm {
    selected: Option<(Weather, usize, Vec3, bool)>,
    elapsed: f32,
    next_strike: f32,
    flash: f32,
    thunder_at: Option<f32>,
    pub(crate) thunder: u32,
}

impl Default for Storm {
    fn default() -> Self {
        Self {
            selected: None,
            elapsed: 0.,
            next_strike: 8.,
            flash: 0.,
            thunder_at: None,
            thunder: 0,
        }
    }
}

impl Storm {
    fn tick(&mut self, active: bool, dt: f32) {
        if active {
            self.elapsed += dt;
            self.flash = (self.flash - dt).max(0.);
            if self.elapsed >= self.next_strike {
                self.flash = 0.18;
                self.thunder_at = Some(self.elapsed + 1.2);
                // Spaced single flashes, never a strobe. Deterministic for captures.
                self.next_strike += 14. + (self.thunder % 4) as f32 * 2.3;
            }
            if self.thunder_at.is_some_and(|at| self.elapsed >= at) {
                self.thunder = self.thunder.wrapping_add(1);
                self.thunder_at = None;
            }
        } else {
            self.flash = 0.;
        }
    }
}

fn cubemap(bytes: &[u8]) -> Image {
    let mut image = Image::from_buffer(
        bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::linear(),
        RenderAssetUsages::default(),
    )
    .expect("embedded skybox PNG is valid");
    image
        .reinterpret_stacked_2d_as_array(6)
        .expect("six square cubemap faces");
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

pub fn setup(commands: &mut Commands, images: &mut Assets<Image>, sun: Vec3) {
    let bytes: [[&[u8]; 3]; 2] = [
        [
            include_bytes!("../sky/clear-128.png"),
            include_bytes!("../sky/clear-256.png"),
            include_bytes!("../sky/clear-512.png"),
        ],
        [
            include_bytes!("../sky/storm-128.png"),
            include_bytes!("../sky/storm-256.png"),
            include_bytes!("../sky/storm-512.png"),
        ],
    ];
    commands.insert_resource(Skies(
        bytes.map(|levels| levels.map(|bytes| images.add(cubemap(bytes)))),
    ));
    commands.init_resource::<Storm>();
    commands.insert_resource(GlobalAmbientLight {
        brightness: 650.,
        ..default()
    });
    commands.insert_resource(DirectionalLightShadowMap { size: 4096 });
    commands.spawn((
        Sun,
        DirectionalLight {
            illuminance: 65_000.,
            shadow_maps_enabled: true,
            contact_shadows_enabled: true,
            shadow_depth_bias: 0.01,
            shadow_normal_bias: 0.3,
            ..default()
        },
        Transform::IDENTITY.looking_to(sun, Vec3::Y),
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 20.,
            maximum_distance: 90.,
            ..default()
        }
        .build(),
        bevy::camera::visibility::RenderLayers::from_layers(&[
            0,
            crate::assets::PLAYER_SHADOW_LAYER,
        ]),
    ));
    commands.spawn((
        Camera3d::default(),
        bevy_egui::PrimaryEguiContext,
        Msaa::Off,
        ShadowFilteringMethod::Gaussian,
        MipBias(0.),
        Transform::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: 2500.,
            ..default()
        }),
        Skybox::default(),
        AmbientLight {
            brightness: 650.,
            ..default()
        },
        Exposure { ev100: 12.6 },
        Tonemapping::AcesFitted,
    ));
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    frontend: Res<Frontend>,
    session: Res<Session>,
    time: Res<Time>,
    skies: Res<Skies>,
    mut storm: ResMut<Storm>,
    mut sun: Query<&mut DirectionalLight, With<Sun>>,
    mut cameras: Query<
        (Entity, &mut Skybox, &mut AmbientLight, &mut Exposure),
        With<bevy_egui::PrimaryEguiContext>,
    >,
) {
    let weather = frontend.draft.weather;
    let quality = match frontend.video.sky {
        Level::Off | Level::Low => 0,
        Level::Medium => 1,
        Level::High | Level::Ultra => 2,
    };
    let direction = session.race.track.sun_direction;
    let direction = Vec3::new(direction.x as f32, direction.y as f32, direction.z as f32);
    let selected = (
        weather,
        quality,
        direction,
        frontend.video.cinematic_lighting,
    );
    if storm.selected != Some(selected) {
        storm.selected = Some(selected);
        storm.elapsed = 0.;
        storm.next_strike = 8.;
        storm.flash = 0.;
        storm.thunder_at = None;
        let handle = skies.0[usize::from(weather == Weather::Storm)][quality].clone();
        // The pinned clear panorama's sun azimuth is 0.6289321 radians.
        // Rotate both the visible sky and its reflection map with the track sun.
        let rotation = Quat::from_rotation_y((-direction.x).atan2(-direction.z) - 0.628_932_1);
        for (entity, mut skybox, _, _) in &mut cameras {
            skybox.image = Some(handle.clone());
            skybox.rotation = rotation;
            commands
                .entity(entity)
                .insert(GeneratedEnvironmentMapLight {
                    environment_map: handle.clone(),
                    // Reflections must be no brighter than the visible sky,
                    // or dark glossy parts (carbon, tyres) mirror a sky
                    // brighter than the real one and turn grey.
                    intensity: weather.brightness(),
                    rotation,
                    ..default()
                });
        }
    }
    storm.tick(
        weather == Weather::Storm && session.ready && !session.paused,
        time.delta_secs(),
    );
    let flash = (storm.flash / 0.18).min(1.);
    let (illuminance, ambient_brightness, color) = match weather {
        Weather::Clear => {
            if frontend.video.cinematic_lighting {
                (70_000., 650., Color::WHITE)
            } else {
                (65_000., 650., Color::WHITE)
            }
        }
        // Ambient light also lights reflections, so it stays low: a strong
        // flat fill puts a white sheen on every glossy panel and turns dark
        // carbon light grey and Ferrari red pink.
        Weather::Storm => (7_000. + flash * 18_000., 500., Color::srgb(0.72, 0.79, 1.)),
    };
    for mut light in &mut sun {
        light.illuminance = illuminance;
        light.color = color;
    }
    for (_, mut skybox, mut ambient, mut exposure) in &mut cameras {
        // A brighter storm exposure pushes saturated paint into the
        // tonemapper's shoulder, where Ferrari red turns pink.
        exposure.ev100 = if weather == Weather::Storm {
            11.9
        } else {
            12.6
        };
        ambient.brightness = ambient_brightness;
        skybox.brightness = weather.brightness() * (1. + flash * 1.5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_skies_have_six_square_faces_at_each_quality() {
        for (bytes, width) in [
            (include_bytes!("../sky/clear-128.png").as_slice(), 128),
            (include_bytes!("../sky/storm-128.png").as_slice(), 128),
            (include_bytes!("../sky/clear-256.png").as_slice(), 256),
            (include_bytes!("../sky/storm-256.png").as_slice(), 256),
            (include_bytes!("../sky/clear-512.png").as_slice(), 512),
            (include_bytes!("../sky/storm-512.png").as_slice(), 512),
        ] {
            let image = cubemap(bytes);
            let size = image.texture_descriptor.size;
            assert_eq!(size.width, width);
            assert_eq!(size.width, size.height);
            assert_eq!(size.depth_or_array_layers, 6);
            assert_eq!(
                image.texture_view_descriptor.unwrap().dimension,
                Some(TextureViewDimension::Cube)
            );
        }
    }

    #[test]
    fn storm_delays_thunder_and_does_not_advance_while_paused() {
        let mut storm = Storm::default();
        storm.tick(true, 8.);
        assert!(storm.flash > 0.);
        assert_eq!(storm.thunder, 0);
        storm.tick(false, 100.);
        assert_eq!(storm.elapsed, 8.);
        assert_eq!(storm.thunder, 0);
        storm.tick(true, 1.3);
        assert_eq!(storm.thunder, 1);
        assert_eq!(storm.flash, 0.);
        assert!(storm.next_strike > 20.);
    }

    #[test]
    fn weather_cli_is_strict_and_old_preferences_default_to_clear() {
        for weather in Weather::ALL {
            assert_eq!(weather.cli().parse::<Weather>().unwrap(), weather);
        }
        assert!("rain".parse::<Weather>().is_err());
        let options =
            crate::options::Options::parse(["--weather", "storm"].map(String::from).into_iter())
                .unwrap();
        assert_eq!(options.weather, Weather::Storm);
        let mut json =
            serde_json::to_value(crate::preferences::Preferences::defaults(&options)).unwrap();
        json.as_object_mut().unwrap().remove("weather");
        let restored: crate::preferences::Preferences = serde_json::from_value(json).unwrap();
        assert_eq!(restored.weather, Weather::Clear);
    }
}
