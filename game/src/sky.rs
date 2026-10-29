//! Bevy's daylight atmosphere and environment lighting.

use bevy::{
    anti_alias::taa::TemporalAntiAliasing,
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    light::{
        atmosphere::ScatteringMedium, Atmosphere, AtmosphereEnvironmentMapLight,
        CascadeShadowConfigBuilder, DirectionalLightShadowMap, ShadowFilteringMethod, SunDisk,
    },
    pbr::{AtmosphereSettings, ContactShadows, ScreenSpaceAmbientOcclusion},
    prelude::*,
    render::camera::MipBias,
};

pub fn setup(commands: &mut Commands, media: &mut Assets<ScatteringMedium>, sun: Vec3) {
    commands.insert_resource(GlobalAmbientLight {
        brightness: 650.0,
        ..default()
    });
    commands.insert_resource(DirectionalLightShadowMap { size: 4096 });
    commands.spawn(Atmosphere::earth(
        media.add(ScatteringMedium::earth(128, 128)),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 65_000.0,
            shadow_maps_enabled: true,
            contact_shadows_enabled: true,
            shadow_depth_bias: 0.01,
            shadow_normal_bias: 0.6,
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
        Msaa::Off,
        TemporalAntiAliasing::default(),
        ContactShadows::default(),
        ScreenSpaceAmbientOcclusion {
            radius: 0.32,
            constant_object_thickness: 0.12,
            ..default()
        },
        ShadowFilteringMethod::Gaussian,
        // TAA's default negative bias brings back the fine texture noise we are filtering.
        MipBias(0.0),
        Transform::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: 2500.0,
            ..default()
        }),
        AtmosphereSettings::default(),
        AtmosphereEnvironmentMapLight::default(),
        Exposure { ev100: 12.6 },
        Tonemapping::AcesFitted,
    ));
}
