//! Graphics presets. Ultra is the full look; lower presets trade the costliest
//! screen-space effects and shadow resolution for frame rate.

use bevy::{
    anti_alias::{fxaa::Fxaa, taa::TemporalAntiAliasing},
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass},
    light::{
        AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, DirectionalLightShadowMap,
        ShadowFilteringMethod,
    },
    pbr::{ContactShadows, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel},
    prelude::*,
    render::camera::TemporalJitter,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

impl Quality {
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Ultra];

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Low => "Basic shadows and FXAA. For integrated graphics.",
            Self::Medium => "Temporal antialiasing, no screen-space effects.",
            Self::High => "Ambient occlusion and contact shadows at balanced resolution.",
            Self::Ultra => "Highest shadow resolution and filtering.",
        }
    }

    fn shadow_map(self) -> usize {
        match self {
            Self::Low => 1024,
            Self::Medium | Self::High => 2048,
            Self::Ultra => 4096,
        }
    }

    fn cascades(self) -> usize {
        match self {
            Self::Low => 2,
            Self::Medium | Self::High => 3,
            Self::Ultra => 4,
        }
    }

    fn ambient_occlusion(self) -> Option<ScreenSpaceAmbientOcclusionQualityLevel> {
        match self {
            Self::Low | Self::Medium => None,
            Self::High => Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
            Self::Ultra => Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
        }
    }
}

/// Applies the preset whenever it changes, including on the first frame.
pub fn apply(
    mut commands: Commands,
    frontend: Res<crate::menu::Frontend>,
    mut applied: Local<Option<Quality>>,
    cameras: Query<Entity, With<Camera3d>>,
    mut lights: Query<(Entity, &mut DirectionalLight)>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    let quality = frontend.graphics;
    if *applied == Some(quality) || cameras.is_empty() {
        return;
    }
    *applied = Some(quality);
    shadow_map.size = quality.shadow_map();
    for (entity, mut light) in &mut lights {
        light.contact_shadows_enabled = quality >= Quality::High;
        commands.entity(entity).insert(
            CascadeShadowConfigBuilder {
                num_cascades: quality.cascades(),
                first_cascade_far_bound: 20.0,
                maximum_distance: 90.0,
                ..default()
            }
            .build(),
        );
    }
    for camera in &cameras {
        let mut camera = commands.entity(camera);
        // Start from a clean slate so prepasses only run when an effect needs them.
        camera.remove::<(
            TemporalAntiAliasing,
            TemporalJitter,
            Fxaa,
            ScreenSpaceAmbientOcclusion,
            ContactShadows,
            AtmosphereEnvironmentMapLight,
            DepthPrepass,
            NormalPrepass,
            MotionVectorPrepass,
        )>();
        camera.insert(if quality == Quality::Low {
            ShadowFilteringMethod::Hardware2x2
        } else {
            ShadowFilteringMethod::Gaussian
        });
        // Sky lighting is cheap at its default size and the scene is dull without it.
        camera.insert(AtmosphereEnvironmentMapLight::default());
        if quality == Quality::Low {
            camera.insert(Fxaa::default());
        } else {
            camera.insert(TemporalAntiAliasing::default());
        }
        if let Some(quality_level) = quality.ambient_occlusion() {
            camera.insert((
                ScreenSpaceAmbientOcclusion {
                    quality_level,
                    radius: 0.32,
                    constant_object_thickness: 0.12,
                },
                ContactShadows::default(),
            ));
        }
    }
}
