//! Live cockpit steering wheel: the GEN22 LCD shows an egui-painted dash and
//! its LED strip lights per lamp, both driven by the player's car. Wheels
//! modelled without a display (the RB6) get a slim screen pod on top.

use crate::{assets::SteeringWheelVisual, hud, Session};
use bevy::{
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, RenderTarget},
    core_pipeline::{
        mip_generation::{
            generate_mips_for_phase, MipGenerationJobs, MipGenerationPhaseId,
            MipGenerationPipelines,
        },
        upscaling::upscaling,
        Core2d,
    },
    ecs::schedule::ScheduleLabel,
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    math::Affine2,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::{
            Extent3d, PipelineCache, TextureDimension, TextureFormat, TextureUsages,
        },
        renderer::RenderContext,
        texture::GpuImage,
        Extract, RenderApp,
    },
};
use bevy_egui::{egui, EguiContext, EguiSchedule, PrimaryEguiContext};

/// Imported LCD and LED-strip primitives on a car's wheel; only the player's
/// are made live, AI wheels keep their imported look.
#[derive(Component)]
pub enum WheelPart {
    Screen(usize),
    Leds(usize),
    /// A wheel rim with no display of its own.
    Mount(usize),
}

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WheelDashPass;

/// The screen is drawn about a tenth of its texture size in the cockpit, so it
/// is sampled from a mip chain rebuilt every frame; without one, thin strokes
/// break up and shimmer as the digits change.
pub struct WheelDashPlugin;

const MIP_PHASE: MipGenerationPhaseId = MipGenerationPhaseId(0x5745_4c44);

impl Plugin for WheelDashPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(ExtractSchedule, extract_screen)
            .add_systems(Core2d, build_screen_mips.after(upscaling));
    }
}

/// Render-world copy of the two screen images.
#[derive(Resource)]
struct ScreenImages {
    target: AssetId<Image>,
    sampled: AssetId<Image>,
}

fn extract_screen(
    mut commands: Commands,
    dash: Extract<Option<Res<WheelDash>>>,
    mut jobs: ResMut<MipGenerationJobs>,
) {
    let Some(dash) = dash.as_ref() else {
        return;
    };
    jobs.add(MIP_PHASE, dash.screen.id());
    commands.insert_resource(ScreenImages {
        target: dash.target.id(),
        sampled: dash.screen.id(),
    });
}

/// After the wheel camera has written its frame, copy it into the sampled
/// texture's top level and downsample the rest. The wheel camera is the only
/// 2D camera, and it renders before the scene camera that samples the screen.
fn build_screen_mips(
    images: Option<Res<ScreenImages>>,
    jobs: Res<MipGenerationJobs>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Option<Res<MipGenerationPipelines>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let (Some(images), Some(pipelines)) = (images, pipelines) else {
        return;
    };
    let (Some(target), Some(sampled)) = (
        gpu_images.get(images.target),
        gpu_images.get(images.sampled),
    ) else {
        return;
    };
    ctx.command_encoder().copy_texture_to_texture(
        target.texture.as_image_copy(),
        sampled.texture.as_image_copy(),
        target.texture.size(),
    );
    generate_mips_for_phase(
        MIP_PHASE,
        &jobs,
        &pipeline_cache,
        &pipelines,
        &gpu_images,
        &mut ctx,
    );
}

#[derive(Resource)]
pub struct WheelDash {
    /// What the wheel camera renders into: one level.
    target: Handle<Image>,
    /// What the LCD samples: the same pixels with a full mip chain.
    screen: Handle<Image>,
    /// The player's wheel uses the pod's strip layout instead of the full LCD.
    compact: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lamp {
    Rev(usize),
}

#[derive(Component)]
pub struct WheelLamp {
    lamp: Lamp,
    lit: Option<Color>,
    off: Color,
}

const REV_LAMPS: usize = 15;

pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let (width, height) = hud::wheel::SIZE;
    // Linear half floats end to end: egui writes linear colour, the
    // downsampler needs a storage-capable format, and averaging is correct.
    let mut target = Image::new_target_texture(width, height, TextureFormat::Rgba16Float, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(target);
    let mut screen = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    screen.texture_descriptor.mip_level_count = width.max(height).ilog2() + 1;
    screen.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::STORAGE_BINDING;
    screen.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        // The LCD is seen at a slant whenever the wheel is turned.
        anisotropy_clamp: 8,
        ..default()
    });
    let screen = images.add(screen);
    // Draws nothing of the world; it only hosts the wheel's egui context.
    commands.spawn((
        Camera2d,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
        // A float main texture so egui's pipeline matches the float target;
        // LCD colours pass through untoned.
        bevy::camera::Hdr,
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        RenderLayers::none(),
        EguiSchedule::new(WheelDashPass),
    ));
    commands.insert_resource(WheelDash {
        target,
        screen,
        compact: false,
    });
}

pub fn draw(
    mut context: Single<&mut EguiContext, Without<PrimaryEguiContext>>,
    style: Res<hud::HudStyle>,
    session: Res<Session>,
    dash: Res<WheelDash>,
    mut initialized: Local<bool>,
) {
    let context = context.get_mut();
    if !*initialized {
        context.set_fonts(hud::fonts());
        *initialized = true;
        return;
    }
    if !session.ready {
        return;
    }
    let canvas = crate::canvas::Hud {
        painter: context.layer_painter(egui::LayerId::background()),
        size: context.content_rect().size(),
        finish: crate::canvas::Finish::Digital,
    };
    if dash.compact {
        hud::wheel::draw_compact(&canvas, &style, &session);
    } else {
        hud::wheel::draw(&canvas, &style, &session);
    }
}

type WheelPartItem = (
    Entity,
    &'static WheelPart,
    &'static Mesh3d,
    &'static MeshMaterial3d<StandardMaterial>,
    &'static Transform,
    Option<&'static SteeringWheelVisual>,
    &'static ChildOf,
);

/// Moves the player's LCD onto the live texture and splits its LED strip into
/// individually lit lamps.
pub fn attach(
    mut commands: Commands,
    parts: Query<WheelPartItem>,
    session: Res<Session>,
    mut dash: ResMut<WheelDash>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, part, mesh, material, transform, visual, parent) in &parts {
        commands.entity(entity).remove::<WheelPart>();
        let (WheelPart::Screen(entrant) | WheelPart::Leds(entrant) | WheelPart::Mount(entrant)) =
            *part;
        if entrant != session.player {
            continue;
        }
        let Some(source) = meshes.get(&mesh.0).cloned() else {
            continue;
        };
        match part {
            WheelPart::Mount(_) => {
                let Some(visual) = visual else { continue };
                dash.compact = true;
                let mut pod = |mesh: Mesh, material: StandardMaterial| {
                    (
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(materials.add(material)),
                        *transform,
                        SteeringWheelVisual {
                            entrant,
                            rest: visual.rest,
                            pivot: visual.pivot,
                        },
                        ChildOf(parent.parent()),
                    )
                };
                // Sits in the open slot above the button plate, whose
                // driver-facing surface is at z ≈ 0.685 in the RB6 model.
                let housing = pod(
                    Cuboid::new(0.124, 0.032, 0.012)
                        .mesh()
                        .build()
                        .translated_by(Vec3::new(0., 0.597, 0.6865)),
                    StandardMaterial {
                        base_color: Color::srgb(0.03, 0.03, 0.035),
                        perceptual_roughness: 0.45,
                        ..default()
                    },
                );
                let screen = pod(
                    screen_quad(Vec2::new(0.108, 0.022), Vec3::new(0., 0.597, 0.6802)),
                    StandardMaterial {
                        base_color_texture: Some(dash.screen.clone()),
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        uv_transform: Affine2::from_scale(Vec2::new(
                            1.,
                            hud::wheel::COMPACT_HEIGHT / hud::wheel::SIZE.1 as f32,
                        )),
                        ..default()
                    },
                );
                commands.spawn(housing);
                commands.spawn(screen);
            }
            WheelPart::Screen(_) => {
                dash.compact = false;
                let Some(window) = screen_uvs(&source) else {
                    continue;
                };
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(dash.screen.clone()),
                        unlit: true,
                        uv_transform: Affine2::from_scale(Vec2::ONE / window.size())
                            * Affine2::from_translation(-window.min),
                        ..default()
                    })));
            }
            WheelPart::Leds(_) => {
                let Some(mut base) = materials.get(&material.0).cloned() else {
                    continue;
                };
                base.base_color = Color::srgb(0.015, 0.018, 0.02);
                base.emissive = LinearRgba::BLACK;
                let (lamps, rest) = split_uv_lamps(&source);
                for (lamp, indices) in lamps {
                    let mut lamp_mesh = source.clone();
                    lamp_mesh.insert_indices(Indices::U32(indices));
                    let mut lamp_entity = commands.spawn((
                        Mesh3d(meshes.add(lamp_mesh)),
                        MeshMaterial3d(materials.add(base.clone())),
                        *transform,
                        WheelLamp {
                            lamp,
                            lit: None,
                            off: base.base_color,
                        },
                        ChildOf(parent.parent()),
                    ));
                    if let Some(visual) = visual {
                        lamp_entity.insert(SteeringWheelVisual {
                            entrant,
                            rest: visual.rest,
                            pivot: visual.pivot,
                        });
                    }
                }
                let mut rest_mesh = source;
                rest_mesh.insert_indices(Indices::U32(rest));
                commands.entity(entity).insert((
                    Mesh3d(meshes.add(rest_mesh)),
                    MeshMaterial3d(materials.add(base)),
                ));
            }
        }
    }
}

/// Driver-facing quad (normal −Z) whose texture's left edge is the driver's
/// left, +X in car space.
fn screen_quad(size: Vec2, centre: Vec3) -> Mesh {
    let (x, y) = (size.x * 0.5, size.y * 0.5);
    let corners = [(x, y), (-x, y), (-x, -y), (x, -y)];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        corners
            .iter()
            .map(|&(dx, dy)| [centre.x + dx, centre.y + dy, centre.z])
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., -1.]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 2, 3, 0]))
}

/// The LCD quad is two-sided; its driver-facing side uses non-negative V.
fn screen_uvs(mesh: &Mesh) -> Option<Rect> {
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let mut window = Rect::EMPTY;
    for uv in uvs.iter().filter(|uv| uv[1] >= 0.) {
        window = window.union_point(Vec2::from_array(*uv));
    }
    (window.width() > 0. && window.height() > 0.).then_some(window)
}

/// Only the fifteen lamps in the narrow top row are rev lights. The imported
/// material also contains side warning lamps; its full-width UVs span both.
fn split_uv_lamps(mesh: &Mesh) -> (Vec<(Lamp, Vec<u32>)>, Vec<u32>) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return (Vec::new(), Vec::new());
    };
    let indices: Vec<u32> = mesh.indices().map_or_else(
        || (0..positions.len() as u32).collect(),
        |indices| indices.iter().map(|i| i as u32).collect(),
    );
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for position in positions {
        let p = Vec3::from_array(*position);
        low = low.min(p);
        high = high.max(p);
    }
    let middle = (low.x + high.x) * 0.5;
    // The warning lamps sit wider than the strip or below it. Coordinates are
    // wheel-local metres; this selection is independent of texture UV layout.
    // The exported row curves down at both ends: its lowest vertices are
    // 4.84 mm below the crown, so allow 6 mm to retain complete lamp lenses.
    let is_rev = |p: &[f32; 3]| p[1] >= high.y - 0.006 && (p[0] - middle).abs() < 0.05;
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for p in positions.iter().filter(|p| is_rev(p)) {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
    }
    if max_x <= min_x {
        return (Vec::new(), indices);
    }
    let mut groups = vec![Vec::new(); REV_LAMPS];
    let mut rest = Vec::new();
    for triangle in indices.chunks_exact(3) {
        if !triangle.iter().all(|i| is_rev(&positions[*i as usize])) {
            rest.extend_from_slice(triangle);
            continue;
        }
        let x = triangle
            .iter()
            .map(|i| positions[*i as usize][0])
            .sum::<f32>()
            / 3.;
        // +X is the driver's left: fill from that end of the strip.
        let u = (max_x - x) / (max_x - min_x);
        let lamp = ((u.clamp(0., 1.) * REV_LAMPS as f32) as usize).min(REV_LAMPS - 1);
        groups[lamp].extend_from_slice(triangle);
    }
    (
        groups
            .into_iter()
            .enumerate()
            .filter(|(_, indices)| !indices.is_empty())
            .map(|(i, indices)| (Lamp::Rev(i), indices))
            .collect(),
        rest,
    )
}

/// Rev lights fill green, red, then blue; during the start they mirror the gantry.
pub fn lights(
    session: Res<Session>,
    mut lamps: Query<(&mut WheelLamp, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !session.ready || lamps.is_empty() {
        return;
    }
    for (mut state, material) in &mut lamps {
        let lit = match state.lamp {
            Lamp::Rev(index) => hud::wheel::rev_light(&session, index, REV_LAMPS).map(|color| {
                let [r, g, b, _] = color.to_array();
                Color::srgb_u8(r, g, b)
            }),
        };
        if lit == state.lit {
            continue;
        }
        state.lit = lit;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.unlit = lit.is_some();
            material.base_color = lit.unwrap_or(state.off);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rev_strip_excludes_side_warning_lamps_and_preserves_their_geometry() {
        let mut positions = Vec::new();
        for i in 0..REV_LAMPS {
            let x = 0.042 - i as f32 * 0.006;
            // Match the exported curved row: outer lens bottoms are about
            // 4.9 mm below the middle lamp's crown.
            let drop = 0.001 * (x / 0.042).powi(2);
            positions.extend([
                [x - 0.001, 0.0888 - drop, -0.024],
                [x + 0.001, 0.0888 - drop, -0.024],
                [x, 0.0927 - drop, -0.024],
            ]);
        }
        for (x, y) in [
            (0.086, 0.088),
            (-0.086, 0.088),
            (0.055, 0.076),
            (-0.055, 0.076),
        ] {
            positions.extend([
                [x - 0.001, y, -0.024],
                [x + 0.001, y, -0.024],
                [x, y + 0.001, -0.024],
            ]);
        }
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        let (lamps, rest) = split_uv_lamps(&mesh);
        assert_eq!(lamps.len(), REV_LAMPS);
        for (i, (lamp, indices)) in lamps.iter().enumerate() {
            assert!(*lamp == Lamp::Rev(i));
            assert_eq!(
                *indices,
                vec![i as u32 * 3, i as u32 * 3 + 1, i as u32 * 3 + 2]
            );
        }
        assert_eq!(rest, (45..57).collect::<Vec<_>>());
    }
}
