//! Bevy's daylight atmosphere and environment lighting.

use bevy::{
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    light::{
        atmosphere::ScatteringMedium, Atmosphere, AtmosphereEnvironmentMapLight,
        CascadeShadowConfigBuilder, SunDisk,
    },
    pbr::AtmosphereSettings,
    prelude::*,
};

pub fn setup(commands: &mut Commands, media: &mut Assets<ScatteringMedium>, sun: Vec3) {
    commands.insert_resource(GlobalAmbientLight {
        brightness: 80.0,
        ..default()
    });
    commands.spawn(Atmosphere::earth(
        media.add(ScatteringMedium::earth(128, 128)),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 100_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::IDENTITY.looking_to(sun, Vec3::Y),
        CascadeShadowConfigBuilder {
            first_cascade_far_bound: 20.0,
            maximum_distance: 90.0,
            ..default()
        }
        .build(),
        SunDisk::EARTH,
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: 2500.0,
            ..default()
        }),
        AtmosphereSettings::default(),
        AtmosphereEnvironmentMapLight::default(),
        Exposure { ev100: 13.0 },
        Tonemapping::AcesFitted,
    ));
}
