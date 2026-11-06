//! Isolated studio view of the selected car, rendered into the setup menu.
use super::{Frontend, Screen, Tab};
use crate::Session;
use bevy::{
    asset::{LoadState, RecursiveDependencyLoadState},
    camera::{visibility::RenderLayers, Exposure, RenderTarget, ScalingMode},
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::render_resource::TextureFormat,
    world_serialization::{WorldAsset, WorldInstanceReady},
};
use bevy_egui::{egui, EguiTextureHandle, EguiUserTextures};
use racecraft_sim::Compound;

const LAYER: usize = 2;
const SIZE: (u32, u32) = (1200, 620);

#[derive(Component)]
pub(crate) struct PreviewCamera;

#[derive(Resource)]
pub(crate) struct CarPreview {
    texture: egui::TextureId,
    selected: Option<(String, Compound, String, String)>,
    root: Option<Entity>,
    asset: Option<Handle<WorldAsset>>,
    textures: Vec<Handle<Image>>,
    livery_path: Option<String>,
    scene_ready: bool,
    pub(super) ready: bool,
    failed: bool,
    retry_at: f64,
    image_visible: std::sync::atomic::AtomicBool,
}

impl CarPreview {
    #[cfg(test)]
    pub(super) fn test_stub() -> Self {
        Self {
            texture: egui::TextureId::default(),
            selected: None,
            root: None,
            asset: None,
            textures: Vec::new(),
            livery_path: None,
            scene_ready: false,
            ready: false,
            failed: false,
            retry_at: 0.,
            image_visible: std::sync::atomic::AtomicBool::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn root(&self) -> Option<Entity> {
        self.root
    }

    #[cfg(test)]
    pub(super) fn selected_car(&self) -> Option<&str> {
        self.selected.as_ref().map(|(car, ..)| car.as_str())
    }

    #[cfg(test)]
    pub(super) fn applied_livery_path(&self) -> Option<&str> {
        self.livery_path.as_deref()
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        let size = egui::vec2(
            ui.available_width(),
            ui.available_width() * SIZE.1 as f32 / SIZE.0 as f32,
        );
        let response = ui.add(egui::Image::new((self.texture, size)));
        self.image_visible.store(
            ui.is_rect_visible(response.rect),
            std::sync::atomic::Ordering::Relaxed,
        );
        if !self.ready {
            ui.painter().text(
                response.rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.failed {
                    "Car preview unavailable. Check the car assets."
                } else {
                    "Loading car…"
                },
                egui::FontId::proportional(17.),
                super::DIM,
            );
        }
    }
}

pub(crate) fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<EguiUserTextures>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let image = images.add(Image::new_target_texture(
        SIZE.0,
        SIZE.1,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let texture = textures.add_image(EguiTextureHandle::Strong(image.clone()));
    commands.insert_resource(CarPreview {
        texture,
        selected: None,
        root: None,
        asset: None,
        textures: Vec::new(),
        livery_path: None,
        scene_ready: false,
        ready: false,
        failed: false,
        retry_at: 0.,
        image_visible: std::sync::atomic::AtomicBool::new(true),
    });
    commands.spawn((
        PreviewCamera,
        Camera3d::default(),
        Camera {
            order: -2,
            is_active: false,
            clear_color: Color::srgb_u8(9, 10, 12).into(),
            ..default()
        },
        RenderTarget::Image(image.into()),
        RenderLayers::layer(LAYER),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 3.8,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(8., 4.8, 9.).looking_at(Vec3::new(0., 0.5, 0.35), Vec3::Y),
        Exposure { ev100: 10.5 },
        Tonemapping::AcesFitted,
        Msaa::Sample4,
    ));
    for (position, intensity) in [
        (Vec3::new(4., 7., 6.), 4_000_000.),
        (Vec3::new(-5., 3., -4.), 2_000_000.),
    ] {
        commands.spawn((
            PointLight {
                intensity,
                range: 30.,
                radius: 1.5,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_translation(position),
            RenderLayers::layer(LAYER),
        ));
    }
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(200., 200.))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb_u8(22, 23, 27),
            perceptual_roughness: 1.,
            ..default()
        })),
        Transform::from_xyz(0., -0.025, 0.),
        RenderLayers::layer(LAYER),
    ));
}

pub(crate) fn update(
    mut commands: Commands,
    frontend: Res<Frontend>,
    session: Res<Session>,
    server: Res<AssetServer>,
    time: Res<Time>,
    mut preview: ResMut<CarPreview>,
    mut cameras: Query<&mut Camera, With<PreviewCamera>>,
) {
    let visible = matches!(session.screen, Screen::Main | Screen::Paused)
        && frontend.tab == Tab::Car
        && frontend.benchmark.child.is_none();
    for mut camera in &mut cameras {
        camera.is_active = visible
            && (!preview.ready
                || preview
                    .image_visible
                    .load(std::sync::atomic::Ordering::Relaxed));
    }
    if !visible {
        return;
    }
    let selected = (
        frontend.draft.car.clone(),
        frontend.draft.setup.compound,
        frontend.draft.team.clone(),
        frontend.draft.driver.clone(),
    );
    let retry = preview.failed && time.elapsed_secs_f64() >= preview.retry_at;
    if retry {
        server.reload(format!("cars/{}/car.glb", frontend.draft.car));
        for texture in &preview.textures {
            if let Some(path) = server.get_path(texture.id()) {
                server.reload(path);
            }
        }
    }
    if retry
        || preview
            .selected
            .as_ref()
            .is_none_or(|previous| *previous != selected)
    {
        if let Some(root) = preview.root.take() {
            commands.entity(root).despawn();
        }
        let scene = server.load(
            GltfAssetLabel::Scene(0).from_asset(format!("cars/{}/car.glb", frontend.draft.car)),
        );
        preview.root = Some(
            commands
                .spawn((WorldAssetRoot(scene.clone()), Transform::default()))
                .observe(ready)
                .id(),
        );
        preview.asset = Some(scene);
        preview.selected = Some(selected);
        preview.ready = false;
        preview.scene_ready = false;
        preview.textures.clear();
        preview.livery_path = None;
        preview.failed = false;
        preview.retry_at = time.elapsed_secs_f64() + 5.;
    }
    preview.failed = preview.asset.as_ref().is_some_and(|asset| {
        matches!(
            server.get_load_state(asset.id()),
            Some(LoadState::Failed(_))
        ) || matches!(
            server.get_recursive_dependency_load_state(asset.id()),
            Some(RecursiveDependencyLoadState::Failed(_))
        )
    }) || preview.textures.iter().any(|texture| {
        matches!(
            server.get_load_state(texture.id()),
            Some(LoadState::Failed(_))
        )
    });
    preview.ready = preview.scene_ready
        && !preview.failed
        && preview
            .textures
            .iter()
            .all(|texture| server.is_loaded_with_dependencies(texture.id()));
}

#[allow(clippy::too_many_arguments)]
fn ready(
    event: On<WorldInstanceReady>,
    mut commands: Commands,
    children: Query<&Children>,
    surfaces: Query<(
        &bevy::gltf::GltfMaterialName,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    frontend: Res<Frontend>,
    mut preview: ResMut<CarPreview>,
) {
    if preview.root != Some(event.entity) {
        return;
    }
    for entity in children.iter_descendants(event.entity) {
        commands.entity(entity).insert(RenderLayers::layer(LAYER));
        if let Ok((name, handle)) = surfaces.get(entity) {
            if let Some(source) = materials.get(&handle.0) {
                let mut material = source.clone();
                crate::materials::calibrate(&name.0, &mut material);
                if let Some((_, compound, team, driver)) =
                    preview.selected.clone().filter(|(car, ..)| car == "gen22")
                {
                    if name.0 == "tyre" {
                        let texture = server.load(crate::materials::tyre_texture(None, compound));
                        material.base_color_texture = Some(texture.clone());
                        preview.textures.push(texture);
                    } else if let Some(path) = frontend
                        .teams
                        .selection(&team, &driver)
                        .ok()
                        .and_then(|(team, driver)| {
                            crate::materials::livery_texture(
                                &frontend.draft.assets,
                                &crate::teams::Teams::identity(team, driver),
                                // 2048² is plenty for the preview and keeps a
                                // paused race plus preview inside 4 GB GPUs.
                                Some("lod1"),
                                &name.0,
                            )
                        })
                    {
                        if name.0 == "source_paint_main_body_stickers" {
                            preview.livery_path = Some(path.clone());
                        }
                        let texture = server.load(path);
                        material.base_color = Color::WHITE;
                        material.base_color_texture = Some(texture.clone());
                        material.base_color_channel = bevy::mesh::UvChannel::Uv0;
                        preview.textures.push(texture);
                    }
                }
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(material)));
            }
        }
    }
    preview.scene_ready = true;
}
