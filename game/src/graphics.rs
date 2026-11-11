//! Graphics presets and detailed video settings. Presets fill every field of
//! [`Video`]; the menu can then tweak any of them ("Custom").

use bevy::{
    anti_alias::{
        contrast_adaptive_sharpening::ContrastAdaptiveSharpening,
        fxaa::Fxaa,
        smaa::{Smaa, SmaaPreset},
        taa::TemporalAntiAliasing,
    },
    core_pipeline::{
        prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass},
        tonemapping::Tonemapping,
    },
    light::{
        AtmosphereEnvironmentMapLight, CascadeShadowConfigBuilder, DirectionalLightShadowMap,
        ShadowFilteringMethod,
    },
    pbr::{
        ContactShadows, DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion,
        ScreenSpaceAmbientOcclusionQualityLevel,
    },
    post_process::{
        bloom::Bloom,
        dof::{DepthOfField, DepthOfFieldMode},
        effect_stack::{ChromaticAberration, Vignette},
        motion_blur::MotionBlur,
    },
    prelude::*,
    render::{camera::TemporalJitter, view::ColorGrading},
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
            Self::High => "Bloom, motion blur, contact shadows and asphalt detail.",
            Self::Ultra => "Ambient occlusion, sharpest shadows and the richest sky.",
        }
    }

    /// Every detailed setting for this preset.
    pub fn video(self) -> Video {
        use Quality::*;
        Video {
            anti_alias: if self == Low {
                AntiAlias::Fxaa
            } else {
                AntiAlias::Taa
            },
            sharpening: 0.,
            shadow_resolution: match self {
                Low => 1024,
                Medium | High => 2048,
                Ultra => 4096,
            },
            shadow_cascades: match self {
                Low => 2,
                Medium | High => 2,
                Ultra => 4,
            },
            shadow_distance: if self == High { 70. } else { 90. },
            shadow_filter: if self == Low {
                ShadowFilter::Hard
            } else {
                ShadowFilter::Soft
            },
            contact_shadows: self >= High,
            ambient_occlusion: match self {
                Low | Medium => Level::Off,
                High => Level::Off,
                Ultra => Level::High,
            },
            sky: if self == Ultra {
                Level::High
            } else {
                Level::Medium
            },
            // Full-resolution maps add about 800 MB and push Ultra past a 4 GB
            // card once a desktop is using it; they stay a manual choice.
            texture: match self {
                Low => TextureDetail::Quarter,
                Medium | High | Ultra => TextureDetail::Half,
            },
            ai_detail: match self {
                Low => Level::Low,
                Medium | High => Level::Medium,
                Ultra => Level::High,
            },
            saturation: if self >= Medium { 1.05 } else { 1. },
            surface_detail: self >= Medium,
            cinematic_lighting: self >= Medium,
            camera_motion: self >= Medium,
            effects: if self >= High {
                Level::High
            } else if self == Medium {
                Level::Low
            } else {
                Level::Off
            },
            bloom: if self >= High { 0.08 } else { 0. },
            haze: if self >= Medium { 0.018 } else { 0. },
            contrast: if self >= Medium { 1.01 } else { 1. },
            motion_blur: if self >= High { 0.18 } else { 0. },
            ..Video::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AntiAlias {
    Off,
    Fxaa,
    Smaa,
    Taa,
    Msaa4,
}

impl AntiAlias {
    pub const ALL: [Self; 5] = [Self::Off, Self::Fxaa, Self::Smaa, Self::Taa, Self::Msaa4];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Fxaa => "FXAA",
            Self::Smaa => "SMAA",
            Self::Taa => "TAA",
            Self::Msaa4 => "MSAA 4x",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl Level {
    pub const ALL: [Self; 5] = [Self::Off, Self::Low, Self::Medium, Self::High, Self::Ultra];
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowFilter {
    Hard,
    Soft,
    Temporal,
}

impl ShadowFilter {
    pub const ALL: [Self; 3] = [Self::Hard, Self::Soft, Self::Temporal];
    pub fn label(self) -> &'static str {
        match self {
            Self::Hard => "Hard",
            Self::Soft => "Soft",
            Self::Temporal => "Soft (temporal)",
        }
    }
}

/// Texture resolution kept when car and track textures load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureDetail {
    Full,
    Half,
    Quarter,
}

impl TextureDetail {
    pub const ALL: [Self; 3] = [Self::Full, Self::Half, Self::Quarter];
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full (6 GB+ video memory)",
            Self::Half => "Half",
            Self::Quarter => "Quarter",
        }
    }
    /// Top mip levels dropped at load.
    pub fn dropped_mips(self) -> u32 {
        match self {
            Self::Full => 0,
            Self::Half => 1,
            Self::Quarter => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneMap {
    AcesFitted,
    AgX,
    TonyMcMapface,
    BlenderFilmic,
    Reinhard,
    Neutral,
}

impl ToneMap {
    pub const ALL: [Self; 6] = [
        Self::AcesFitted,
        Self::AgX,
        Self::TonyMcMapface,
        Self::BlenderFilmic,
        Self::Reinhard,
        Self::Neutral,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::AcesFitted => "ACES (default)",
            Self::AgX => "AgX",
            Self::TonyMcMapface => "Tony McMapface",
            Self::BlenderFilmic => "Blender Filmic",
            Self::Reinhard => "Reinhard",
            Self::Neutral => "Neutral",
        }
    }
    fn component(self) -> Tonemapping {
        match self {
            Self::AcesFitted => Tonemapping::AcesFitted,
            Self::AgX => Tonemapping::AgX,
            Self::TonyMcMapface => Tonemapping::TonyMcMapface,
            Self::BlenderFilmic => Tonemapping::BlenderFilmic,
            Self::Reinhard => Tonemapping::ReinhardLuminance,
            Self::Neutral => Tonemapping::SomewhatBoringDisplayTransform,
        }
    }
}

/// Detailed video settings. Strength values of 0 turn an effect off.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Video {
    pub anti_alias: AntiAlias,
    /// Contrast-adaptive sharpening, 0..1.
    pub sharpening: f32,
    pub shadow_resolution: usize,
    pub shadow_cascades: usize,
    /// Metres from the camera that receive sun shadows.
    pub shadow_distance: f32,
    pub shadow_filter: ShadowFilter,
    pub contact_shadows: bool,
    pub ambient_occlusion: Level,
    /// Sky lighting table resolution (Off is treated as Low).
    pub sky: Level,
    pub texture: TextureDetail,
    /// How far AI cars keep their nearer model (Low/Medium/High).
    pub ai_detail: Level,
    pub tonemapping: ToneMap,
    /// Exposure offset in stops.
    pub exposure: f32,
    pub saturation: f32,
    pub contrast: f32,
    /// Colour temperature shift, -1 (cool) .. 1 (warm).
    pub temperature: f32,
    pub bloom: f32,
    pub vignette: f32,
    pub chromatic_aberration: f32,
    /// Motion blur shutter angle in turns, 0..1.
    pub motion_blur: f32,
    pub depth_of_field: bool,
    pub bokeh: bool,
    /// Metres the depth of field keeps sharp.
    pub focus_distance: f32,
    /// Atmospheric haze density, 0..1.
    pub haze: f32,
    pub effects: Level,
    pub surface_detail: bool,
    pub cinematic_lighting: bool,
    pub camera_motion: bool,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            anti_alias: AntiAlias::Taa,
            sharpening: 0.,
            shadow_resolution: 2048,
            shadow_cascades: 3,
            shadow_distance: 90.,
            shadow_filter: ShadowFilter::Soft,
            contact_shadows: true,
            ambient_occlusion: Level::Medium,
            sky: Level::Medium,
            texture: TextureDetail::Half,
            ai_detail: Level::Medium,
            tonemapping: ToneMap::AcesFitted,
            exposure: 0.,
            saturation: 1.,
            contrast: 1.,
            temperature: 0.,
            bloom: 0.,
            vignette: 0.,
            chromatic_aberration: 0.,
            motion_blur: 0.,
            depth_of_field: false,
            bokeh: false,
            focus_distance: 30.,
            haze: 0.,
            effects: Level::Off,
            surface_detail: false,
            cinematic_lighting: false,
            camera_motion: true,
        }
    }
}

impl Video {
    /// Clamp values loaded from disk into supported ranges.
    pub fn sanitize(&mut self) {
        fn clean(value: &mut f32, lo: f32, hi: f32, default: f32) {
            *value = if value.is_finite() {
                value.clamp(lo, hi)
            } else {
                default
            };
        }
        self.shadow_resolution = self.shadow_resolution.clamp(512, 8192).next_power_of_two();
        self.shadow_cascades = self.shadow_cascades.clamp(1, 4);
        clean(&mut self.shadow_distance, 30., 300., 90.);
        clean(&mut self.sharpening, 0., 1., 0.);
        clean(&mut self.exposure, -3., 3., 0.);
        clean(&mut self.saturation, 0., 2., 1.);
        clean(&mut self.contrast, 0.5, 1.5, 1.);
        clean(&mut self.temperature, -1., 1., 0.);
        clean(&mut self.bloom, 0., 1., 0.);
        clean(&mut self.vignette, 0., 1., 0.);
        clean(&mut self.chromatic_aberration, 0., 1., 0.);
        clean(&mut self.motion_blur, 0., 1., 0.);
        clean(&mut self.focus_distance, 1., 500., 30.);
        clean(&mut self.haze, 0., 1., 0.);
    }

    /// The preset these settings match exactly, if any.
    pub fn preset(&self) -> Option<Quality> {
        Quality::ALL
            .into_iter()
            .find(|quality| quality.video() == *self)
    }

    /// Metres at which AI cars switch to their distant model.
    pub fn ai_lod_distance(&self) -> f32 {
        match self.ai_detail {
            Level::Off | Level::Low => 20.,
            Level::Medium => crate::assets::OPPONENT_LOD_DISTANCE,
            Level::High | Level::Ultra => 90.,
        }
    }

    fn ambient_occlusion(&self) -> Option<ScreenSpaceAmbientOcclusionQualityLevel> {
        use ScreenSpaceAmbientOcclusionQualityLevel as Q;
        // Bevy's SSAO does not support multisampled targets.
        if self.anti_alias == AntiAlias::Msaa4 {
            return None;
        }
        match self.ambient_occlusion {
            Level::Off => None,
            Level::Low => Some(Q::Low),
            Level::Medium => Some(Q::Medium),
            Level::High => Some(Q::High),
            Level::Ultra => Some(Q::Ultra),
        }
    }

    fn color_grading(&self) -> ColorGrading {
        let mut grading = ColorGrading::default();
        grading.global.exposure = self.exposure;
        // Bevy shifts the CIE x white point directly; keep the slider range subtle.
        grading.global.temperature = self.temperature * 0.05;
        grading.global.post_saturation = self.saturation;
        for section in [
            &mut grading.shadows,
            &mut grading.midtones,
            &mut grading.highlights,
        ] {
            section.contrast = self.contrast;
        }
        grading
    }
}

/// Applies the video settings whenever they change, including on the first frame.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    mut commands: Commands,
    server: Res<AssetServer>,
    frontend: Res<crate::menu::Frontend>,
    mut applied: Local<Option<(Video, bool)>>,
    mut windows: Query<&mut Window>,
    cameras: Query<Entity, (With<Camera3d>, With<bevy_egui::PrimaryEguiContext>)>,
    mut lights: Query<(Entity, &mut DirectionalLight)>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut textures: ResMut<crate::textures::TextureDetail>,
    mut ai_lod: ResMut<crate::assets::OpponentLodDistance>,
) {
    let video = frontend.video;
    if *applied == Some((video, frontend.vsync)) || cameras.is_empty() {
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
    if applied.is_some_and(|(previous, _)| previous == video) {
        *applied = Some((video, frontend.vsync));
        return;
    }
    let texture_changed = applied
        .as_ref()
        .is_some_and(|(previous, _)| previous.texture != video.texture);
    *applied = Some((video, frontend.vsync));
    if texture_changed {
        // Bevy otherwise keeps the old GLB and image handles alive, so a new
        // race would reuse the previous mip selection from the asset cache.
        for path in [
            format!("cars/{}/car.glb", frontend.draft.car),
            "cars/gen22/car_lod1.glb".into(),
            "cars/gen22/car_lod2.glb".into(),
            format!("tracks/{}/visual.glb", frontend.draft.track),
        ] {
            server.reload(path);
        }
    }
    textures.0 = video.texture.dropped_mips();
    ai_lod.set_if_neq(crate::assets::OpponentLodDistance(video.ai_lod_distance()));
    shadow_map.size = video.shadow_resolution;
    for (entity, mut light) in &mut lights {
        light.contact_shadows_enabled =
            video.contact_shadows && video.anti_alias != AntiAlias::Msaa4;
        commands.entity(entity).insert(
            CascadeShadowConfigBuilder {
                num_cascades: video.shadow_cascades,
                first_cascade_far_bound: (video.shadow_distance * 0.12).min(10.0),
                maximum_distance: video.shadow_distance,
                ..default()
            }
            .build(),
        );
    }
    let temporal = video.anti_alias == AntiAlias::Taa;
    for camera in &cameras {
        let mut camera = commands.entity(camera);
        // Start from a clean slate so prepasses only run when an effect needs them.
        camera.remove::<(
            TemporalAntiAliasing,
            TemporalJitter,
            Fxaa,
            Smaa,
            ContrastAdaptiveSharpening,
            ScreenSpaceAmbientOcclusion,
            ContactShadows,
            AtmosphereEnvironmentMapLight,
            DepthPrepass,
            NormalPrepass,
            MotionVectorPrepass,
        )>();
        camera.remove::<(
            Bloom,
            Vignette,
            ChromaticAberration,
            MotionBlur,
            DepthOfField,
            DistanceFog,
        )>();
        camera.insert((
            match video.shadow_filter {
                ShadowFilter::Hard => ShadowFilteringMethod::Hardware2x2,
                ShadowFilter::Temporal if temporal => ShadowFilteringMethod::Temporal,
                _ => ShadowFilteringMethod::Gaussian,
            },
            if video.anti_alias == AntiAlias::Msaa4 {
                Msaa::Sample4
            } else {
                Msaa::Off
            },
            video.tonemapping.component(),
            video.color_grading(),
        ));
        // The selected weather sky supplies matching environment lighting.
        match video.anti_alias {
            AntiAlias::Off | AntiAlias::Msaa4 => {}
            AntiAlias::Fxaa => {
                camera.insert(Fxaa::default());
            }
            AntiAlias::Smaa => {
                camera.insert(Smaa {
                    preset: SmaaPreset::High,
                });
            }
            AntiAlias::Taa => {
                camera.insert(TemporalAntiAliasing::default());
            }
        }
        if video.sharpening > 0. {
            camera.insert(ContrastAdaptiveSharpening {
                enabled: true,
                sharpening_strength: video.sharpening,
                denoise: false,
            });
        }
        if let Some(quality_level) = video.ambient_occlusion() {
            camera.insert(ScreenSpaceAmbientOcclusion {
                quality_level,
                radius: 0.32,
                constant_object_thickness: 0.12,
            });
        }
        if video.contact_shadows && video.anti_alias != AntiAlias::Msaa4 {
            camera.insert(ContactShadows {
                linear_steps: if video.ambient_occlusion >= Level::High {
                    12
                } else {
                    6
                },
                ..default()
            });
        }

        if video.bloom > 0. {
            camera.insert(Bloom {
                intensity: 0.05 + video.bloom * 0.3,
                ..Bloom::NATURAL
            });
        }
        if video.vignette > 0. {
            camera.insert(Vignette {
                intensity: video.vignette,
                ..default()
            });
        }
        if video.chromatic_aberration > 0. {
            camera.insert(ChromaticAberration {
                intensity: video.chromatic_aberration * 0.04,
                ..default()
            });
        }
        if video.motion_blur > 0. {
            camera.insert(MotionBlur {
                shutter_angle: video.motion_blur,
                samples: 4,
            });
        }
        if video.depth_of_field {
            camera.insert(DepthOfField {
                mode: if video.bokeh {
                    DepthOfFieldMode::Bokeh
                } else {
                    DepthOfFieldMode::Gaussian
                },
                focal_distance: video.focus_distance,
                aperture_f_stops: 2.8,
                ..default()
            });
        }
        if video.haze > 0. {
            camera.insert(DistanceFog {
                color: Color::srgba(0.62, 0.70, 0.80, 1.0),
                falloff: FogFalloff::Exponential {
                    density: video.haze * 0.004,
                },
                ..default()
            });
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

/// Filter the selected sky into diffuse/specular lighting until pipelines settle,
/// then keep the generated textures. Weather or sky-quality changes regenerate it.
pub struct StaticSkyLighting;

impl Plugin for StaticSkyLighting {
    fn build(&self, app: &mut App) {
        let settled = PipelinesSettled::default();
        if std::env::var_os("F1_VISUAL_TRACE").is_some() {
            app.add_systems(Last, trace_frames);
        }
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
    (
        Entity,
        Option<Ref<'static, bevy::light::GeneratedEnvironmentMapLight>>,
    ),
    (With<Camera3d>, With<bevy::light::Skybox>),
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
        *generated_frames = 0;
    }
    for (camera, generating) in &cameras {
        if generating.is_none() || !session.ready {
            *generated_frames = 0;
            continue;
        }
        if generating.as_ref().is_some_and(|light| light.is_changed()) {
            *generated_frames = 0;
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

/// Optional hitch evidence; enabled only for a separate diagnostic run.
fn trace_frames(
    settled: Res<PipelinesSettled>,
    time: Res<Time<Real>>,
    session: Res<crate::Session>,
    particles: Query<(), With<crate::effects::Particle>>,
    mut images: MessageReader<AssetEvent<Image>>,
    mut file: Local<Option<std::fs::File>>,
) {
    use std::io::Write;
    let uploads = images
        .read()
        .filter(|event| {
            matches!(
                event,
                AssetEvent::Added { .. } | AssetEvent::Modified { .. }
            )
        })
        .count();
    let frames_ready = settled.0.load(std::sync::atomic::Ordering::Relaxed);
    let file = file.get_or_insert_with(|| {
        std::fs::File::create(std::env::var_os("F1_VISUAL_TRACE").unwrap()).unwrap()
    });
    let value = serde_json::json!({"elapsed":time.elapsed_secs_f64(),"dt_ms":time.delta_secs_f64()*1000.,"pipelines_settled_frames":frames_ready,"image_events":uploads,"particles":particles.iter().count(),"race_time":session.race.time});
    writeln!(file, "{value}").unwrap();
}

/// Reduce the shutter at low speeds; the chased car's motion vectors remain near zero.
pub fn speed_blur(
    session: Res<crate::Session>,
    frontend: Res<crate::menu::Frontend>,
    mut cameras: Query<&mut MotionBlur, With<bevy_egui::PrimaryEguiContext>>,
) {
    let speed = session.race.entrants[session.player].car.speed() as f32;
    for mut blur in &mut cameras {
        blur.shutter_angle = frontend.video.motion_blur * ((speed - 15.) / 60.).clamp(0., 1.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_configs_match_real_presets() {
        let output = std::process::Command::new("python3")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["-c", "import runpy,json; v=runpy.run_path('../tools/verify_visuals.py')['video']; print(json.dumps([v(p,True) for p in ['low','high','ultra']]))"])
            .output().unwrap();
        assert!(output.status.success());
        let configs: Vec<Video> = serde_json::from_slice(&output.stdout).unwrap();
        for (config, preset) in configs
            .iter()
            .zip([Quality::Low, Quality::High, Quality::Ultra])
        {
            assert_eq!(
                *config,
                preset.video(),
                "verification settings drifted for {preset:?}"
            );
        }
    }

    #[test]
    fn presets_round_trip_and_tweaks_become_custom() {
        for quality in Quality::ALL {
            assert_eq!(quality.video().preset(), Some(quality));
        }
        let mut video = Quality::High.video();
        video.bloom = 0.4;
        assert_eq!(video.preset(), None);
    }

    #[test]
    fn sanitize_clamps_loaded_settings_and_msaa_disables_ssao() {
        let mut video = Video {
            shadow_resolution: 3000,
            shadow_cascades: 9,
            exposure: f32::NAN,
            saturation: 5.,
            anti_alias: AntiAlias::Msaa4,
            ambient_occlusion: Level::Ultra,
            ..Video::default()
        };
        video.sanitize();
        assert_eq!(video.shadow_resolution, 4096);
        assert_eq!(video.shadow_cascades, 4);
        assert_eq!(video.exposure, 0.);
        assert_eq!(video.saturation, 2.);
        assert!(video.ambient_occlusion().is_none());
    }
}
