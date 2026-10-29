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
    mut applied: Local<Option<(Quality, bool)>>,
    mut windows: Query<&mut Window>,
    cameras: Query<Entity, With<Camera3d>>,
    mut lights: Query<(Entity, &mut DirectionalLight)>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
) {
    let quality = frontend.graphics;
    if *applied == Some((quality, frontend.vsync)) || cameras.is_empty() {
        return;
    }
    let present_mode = if frontend.vsync {
        bevy::window::PresentMode::AutoVsync
    } else {
        bevy::window::PresentMode::AutoNoVsync
    };
    for mut window in &mut windows {
        if window.present_mode != present_mode {
            window.present_mode = present_mode;
        }
    }
    if applied.is_some_and(|(previous, _)| previous == quality) {
        *applied = Some((quality, frontend.vsync));
        return;
    }
    *applied = Some((quality, frontend.vsync));
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

/// Frames the sky lighting must be generated with every pipeline ready before
/// it is frozen. Covers pipelined rendering's one-frame lag with margin.
const ENVIRONMENT_SETTLE_FRAMES: u32 = 8;

/// Consecutive rendered frames with no pipeline still compiling, shared from
/// the render world.
#[derive(Resource, Clone, Default)]
struct PipelinesSettled(std::sync::Arc<std::sync::atomic::AtomicU32>);

/// The sun and atmosphere never change during a race, yet Bevy re-filters the
/// sky into diffuse and specular lighting every frame (~1 ms of GPU time).
/// Generate it until it has settled, then keep the result as a static
/// environment map. A new sun direction or preset regenerates it.
pub struct StaticSkyLighting;

impl Plugin for StaticSkyLighting {
    fn build(&self, app: &mut App) {
        let settled = PipelinesSettled::default();
        app.insert_resource(settled.clone())
            .add_systems(Update, freeze_sky_lighting.after(apply));
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.insert_resource(settled).add_systems(
                bevy::render::Render,
                track_pipelines.in_set(bevy::render::RenderSystems::Cleanup),
            );
        }
    }
}

fn track_pipelines(
    cache: Res<bevy::render::render_resource::PipelineCache>,
    settled: Res<PipelinesSettled>,
) {
    use std::sync::atomic::Ordering;
    if cache.waiting_pipelines().next().is_some() {
        settled.0.store(0, Ordering::Relaxed);
    } else {
        settled.0.fetch_add(1, Ordering::Relaxed);
    }
}

type SkyCameras<'w, 's> = Query<
    'w,
    's,
    (Entity, Has<bevy::light::GeneratedEnvironmentMapLight>),
    (With<Camera3d>, With<AtmosphereEnvironmentMapLight>),
>;

fn freeze_sky_lighting(
    mut commands: Commands,
    settled: Res<PipelinesSettled>,
    cameras: SkyCameras,
    sun: Query<(), (With<DirectionalLight>, Changed<Transform>)>,
    session: Res<crate::Session>,
    mut generated_frames: Local<u32>,
) {
    if !sun.is_empty() {
        // Re-inserting restarts generation for the new sky.
        for (camera, _) in &cameras {
            commands
                .entity(camera)
                .insert(AtmosphereEnvironmentMapLight::default());
        }
        *generated_frames = 0;
        return;
    }
    for (camera, generating) in &cameras {
        if !generating || !session.ready {
            *generated_frames = 0;
            continue;
        }
        *generated_frames += 1;
        let ready = settled.0.load(std::sync::atomic::Ordering::Relaxed);
        if *generated_frames > ENVIRONMENT_SETTLE_FRAMES && ready > ENVIRONMENT_SETTLE_FRAMES {
            // Keeps the generated EnvironmentMapLight textures; only stops refiltering.
            commands
                .entity(camera)
                .remove::<bevy::light::GeneratedEnvironmentMapLight>();
        }
    }
}
