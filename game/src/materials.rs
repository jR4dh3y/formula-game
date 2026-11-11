//! Bring imported car surfaces into the same daylight response as the track.

use bevy::{gltf::GltfMaterialName, mesh::UvChannel, prelude::*};
use std::collections::{HashMap, HashSet};

#[derive(Component)]
pub(crate) struct CarSurface {
    pub lod: Option<&'static str>,
    pub compound: racecraft_sim::Compound,
    pub entrant: usize,
}

#[derive(Component)]
pub(crate) struct SurfaceReady;

type PendingSurfaces<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GltfMaterialName,
        &'static CarSurface,
        &'static MeshMaterial3d<StandardMaterial>,
    ),
    (With<CarSurface>, Without<SurfaceReady>),
>;

#[allow(clippy::too_many_arguments)]
pub fn prepare(
    mut commands: Commands,
    surfaces: PendingSurfaces,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut prepared: Local<HashSet<AssetId<StandardMaterial>>>,
    mut instances: Local<SurfaceMaterials>,
    server: Res<AssetServer>,
    options: Res<crate::options::Options>,
    session: Res<crate::Session>,
    in_use: Query<&MeshMaterial3d<StandardMaterial>>,
) {
    if !surfaces.is_empty() {
        // A new race replaced the scene: release painted instances (and their
        // livery textures) that no remaining car uses before loading new ones.
        let used: HashSet<_> = in_use.iter().map(|handle| handle.0.id()).collect();
        instances.retain_used(&used);
    }
    for (entity, name, surface, handle) in &surfaces {
        {
            let Some(mut material) = materials.get_mut(&handle.0) else {
                continue;
            };
            if prepared.insert(handle.0.id()) {
                calibrate(&name.0, &mut material);
            }
        }
        if options.car == "gen22" && name.0 == "tyre" {
            let compound = surface.compound.label().to_ascii_lowercase();
            let path = tyre_texture(surface.lod, surface.compound);
            let instance =
                instances.instance(&compound, &handle.0, server.load(path), &mut materials);
            commands.entity(entity).insert(MeshMaterial3d(instance));
        } else if options.car == "gen22" {
            if let Some(path) = session.identities[surface.entrant]
                .as_ref()
                .and_then(|identity| {
                    livery_texture(&options.assets, identity, surface.lod, &name.0)
                })
            {
                let instance =
                    instances.instance(&path, &handle.0, server.load(path.clone()), &mut materials);
                commands.entity(entity).insert(MeshMaterial3d(instance));
            }
        }
        commands.entity(entity).insert(SurfaceReady);
    }
}

/// Driver artwork overrides team artwork; reduced textures fall back to full size.
pub(crate) fn livery_texture(
    root: &std::path::Path,
    identity: &crate::teams::Identity,
    lod: Option<&str>,
    material: &str,
) -> Option<String> {
    let part = material
        .strip_prefix("source_paint_")?
        .split('.')
        .next()?
        .trim_end_matches("_lod");
    let part = if part == "main_body_stickers" {
        "main_body"
    } else {
        part
    };
    let team = format!("cars/gen22/liveries/{}", identity.team);
    for directory in [format!("{team}/{}", identity.driver), team] {
        if let Some(lod) = lod {
            let path = format!("{directory}/{lod}/{part}.png");
            if root.join(&path).is_file() {
                return Some(path);
            }
        }
        let path = format!("{directory}/{part}.png");
        if root.join(&path).is_file() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn tyre_texture(lod: Option<&str>, compound: racecraft_sim::Compound) -> String {
    let compound = compound.label().to_ascii_lowercase();
    let directory = lod.map_or_else(String::new, |lod| format!("{lod}/"));
    format!("cars/gen22/tyres/{directory}{compound}.png")
}

#[derive(Default)]
pub(crate) struct SurfaceMaterials(
    HashMap<(String, AssetId<StandardMaterial>), Handle<StandardMaterial>>,
);

impl SurfaceMaterials {
    fn retain_used(&mut self, used: &HashSet<AssetId<StandardMaterial>>) {
        self.0.retain(|_, instance| used.contains(&instance.id()));
    }

    fn instance(
        &mut self,
        key: &str,
        source: &Handle<StandardMaterial>,
        texture: Handle<Image>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        self.0
            .retain(|(_, source), _| materials.get(*source).is_some());
        self.0
            .entry((key.to_owned(), source.id()))
            .or_insert_with(|| {
                let mut material = materials.get(source).unwrap().clone();
                material.base_color = Color::WHITE;
                material.base_color_texture = Some(texture);
                material.base_color_channel = UvChannel::Uv0;
                materials.add(material)
            })
            .clone()
    }
}

const SATIN_ROUGHNESS: f32 = 0.72;
const TYRE_TINT: f32 = 0.7;
const SATIN_REFLECTANCE: f32 = 0.25;

/// Livery surfaces that are bare or satin carbon on a real car: floor, wings,
/// wheel covers and cockpit padding. Their liveries are mostly black, and a
/// glossy clearcoat makes them mirror the sky and read grey instead.
fn satin_paint(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["floor", "wing", "covers", "cockpit_foam"]
        .iter()
        .any(|part| name.contains(part))
}

pub(crate) fn calibrate(name: &str, material: &mut StandardMaterial) {
    if name.starts_with("source_paint_") && satin_paint(name) {
        material.base_color_texture = None;
        material.base_color = Color::linear_rgb(0.32, 0.34, 0.36);
        material.perceptual_roughness = SATIN_ROUGHNESS;
        material.reflectance = SATIN_REFLECTANCE;
        material.clearcoat = 0.;
        return;
    }
    if name.starts_with("source_paint_") {
        // Neutral stock paint also handles older assets with embedded artwork.
        material.base_color_texture = None;
        material.base_color = Color::linear_rgb(0.32, 0.34, 0.36);
        material.reflectance = 0.4;
        material.clearcoat = 0.12;
        material.clearcoat_perceptual_roughness = 0.2;
        return;
    }
    // Other Gen22 PBR values and texture maps are authored by the exporter.
    if name.starts_with("source_") || name.starts_with("wheel_") {
        return;
    }
    let name = name.to_ascii_lowercase();
    // Imported exporter specular colors can exceed 1.0 (up to 12 on the RB6 visor).
    material.specular_tint = Color::WHITE;
    if name.contains("mirror") {
        material.metallic = 1.;
        material.perceptual_roughness = 0.12;
        material.reflectance = 0.5;
    } else if ["tyre", "tread", "sidewall"]
        .iter()
        .any(|part| name.contains(part))
    {
        material.metallic = 0.;
        material.perceptual_roughness = 0.85;
        material.reflectance = 0.5;
        material.clearcoat = 0.;
    } else if matches!(name.as_str(), "suit" | "rubber" | "brake") {
        material.metallic = 0.;
        material.perceptual_roughness = 0.8;
        material.reflectance = 0.25;
    } else if matches!(name.as_str(), "rim" | "metal") {
        material.metallic = 0.8;
        material.perceptual_roughness = 0.35;
    } else if name.contains("carbon") {
        material.metallic = 0.;
        material.perceptual_roughness = if name == "carbon_gloss" { 0.32 } else { 0.65 };
        material.reflectance = 0.18;
    } else if ["paint", "decals", "number", "redbull_color"]
        .iter()
        .any(|part| name.contains(part))
        || name == "body"
        || name == "body_slice"
    {
        // Painted composite is dielectric, not polished metal. Keep the livery map.
        material.metallic = 0.;
        material.perceptual_roughness = 0.48;
        material.reflectance = 0.25;
        // Legacy body artwork mixes painted and exposed composite regions.
        material.clearcoat = 0.;
    } else if name.contains("glass") {
        material.metallic = 0.;
        material.perceptual_roughness = 0.18;
        material.reflectance = 0.5;
    } else {
        material.perceptual_roughness = material.perceptual_roughness.max(0.4);
        material.reflectance = material.reflectance.clamp(0.2, 0.5);
    }
}

/// Reapply the finish control to both shared sources and per-entrant livery instances.
pub fn finishes(
    frontend: Res<crate::menu::Frontend>,
    surfaces: Query<(&GltfMaterialName, Ref<MeshMaterial3d<StandardMaterial>>), With<CarSurface>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut applied: Local<Option<bool>>,
) {
    let enabled = frontend.video.cinematic_lighting;
    let changed = *applied != Some(enabled);
    *applied = Some(enabled);
    for (name, handle) in &surfaces {
        if !changed && !handle.is_changed() {
            continue;
        }
        let Some(mut material) = materials.get_mut(&handle.0) else {
            continue;
        };
        let name = name.0.to_ascii_lowercase();
        if name.starts_with("source_paint_") && satin_paint(&name) {
            material.clearcoat = 0.;
            material.perceptual_roughness = SATIN_ROUGHNESS;
            material.reflectance = SATIN_REFLECTANCE;
        } else if name.starts_with("source_paint_") {
            // Current liveries are mostly satin paint over carbon: a light
            // lacquer, not a road car's mirror clearcoat that washes the
            // colours out to pastel under a bright sky.
            material.clearcoat = if enabled { 0.12 } else { 0. };
            material.clearcoat_perceptual_roughness = 0.2;
            material.perceptual_roughness = if enabled { 0.34 } else { 0.36 };
            material.reflectance = if enabled { 0.5 } else { 0.4 };
        } else if name == "tyre" {
            // Rubber is matte; a glossy tyre mirrors the sky and reads grey.
            material.perceptual_roughness = 0.85;
            // The baked rubber (about 0.04 linear) reads charcoal next to the
            // asphalt; take it to about 0.028 while the compound stripe stays
            // clearly yellow, white or red.
            material.base_color = Color::linear_rgb(TYRE_TINT, TYRE_TINT, TYRE_TINT);
        } else if name.starts_with("source_carbon") {
            // The authored roughness map (about 0.32) is already a satin
            // carbon. Any multiplier below 1 turns the floor and wings into
            // sky-coloured mirrors, which reads as grey rather than black.
            material.perceptual_roughness = 1.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retain_used_releases_unreferenced_painted_instances() {
        let mut materials = Assets::<StandardMaterial>::default();
        let source = materials.add(StandardMaterial::default());
        let texture = Handle::<Image>::default();
        let mut instances = SurfaceMaterials::default();
        let kept = instances.instance("kept", &source, texture.clone(), &mut materials);
        let removed = instances.instance("removed", &source, texture, &mut materials);
        let used = HashSet::from([kept.id()]);

        instances.retain_used(&used);

        assert!(instances.0.values().any(|handle| handle.id() == kept.id()));
        assert!(!instances
            .0
            .values()
            .any(|handle| handle.id() == removed.id()));
    }

    #[test]
    fn driver_livery_falls_back_by_part_and_lod_without_leaking_to_other_teams() {
        let root = std::env::temp_dir().join(format!("f1-livery-paths-{}", std::process::id()));
        let teams =
            crate::teams::Teams::parse(include_bytes!("../../tools/car/teams.json")).unwrap();
        let (team, driver) = teams.selection("ferrari", "leclerc").unwrap();
        let identity = crate::teams::Teams::identity(team, driver);
        let base = "cars/gen22/liveries/ferrari";
        let team_path = format!("{base}/main_body.png");
        let driver_path = format!("{base}/leclerc/main_body.png");
        let lod1_path = format!("{base}/leclerc/lod1/main_body.png");
        let lod2_path = format!("{base}/leclerc/lod2/main_body.png");
        for path in [&team_path, &driver_path, &lod1_path, &lod2_path] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), []).unwrap();
        }
        assert_eq!(
            livery_texture(
                &root,
                &identity,
                Some("lod2"),
                "source_paint_main_body_stickers_lod.001"
            ),
            Some(lod2_path)
        );
        assert_eq!(
            livery_texture(
                &root,
                &identity,
                Some("lod1"),
                "source_paint_main_body_stickers_lod"
            ),
            Some(lod1_path.clone())
        );
        std::fs::remove_file(root.join(&lod1_path)).unwrap();
        assert_eq!(
            livery_texture(
                &root,
                &identity,
                Some("lod1"),
                "source_paint_main_body_stickers_lod"
            ),
            Some(driver_path.clone())
        );
        std::fs::remove_file(root.join(&driver_path)).unwrap();
        assert_eq!(
            livery_texture(&root, &identity, None, "source_paint_main_body_stickers"),
            Some(team_path)
        );
        assert!(livery_texture(&root, &identity, None, "source_paint_front_wing").is_none());
        assert!(livery_texture(&root, &identity, None, "tyre").is_none());
        let (team, driver) = teams.selection("mclaren", "").unwrap();
        assert!(livery_texture(
            &root,
            &crate::teams::Teams::identity(team, driver),
            None,
            "source_paint_main_body_stickers"
        )
        .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn livery_instances_preserve_pbr_and_do_not_repaint_shared_materials() {
        let mut materials = Assets::<StandardMaterial>::default();
        let normal = Handle::<Image>::default();
        let original = StandardMaterial {
            base_color: Color::linear_rgb(0.32, 0.34, 0.36),
            normal_map_texture: Some(normal.clone()),
            perceptual_roughness: 0.3,
            ..default()
        };
        let source = materials.add(original.clone());
        let mut instances = SurfaceMaterials::default();
        let ferrari = instances.instance("ferrari", &source, Handle::default(), &mut materials);
        let other = instances.instance("other", &source, Handle::default(), &mut materials);
        assert_ne!(ferrari, other);
        assert_eq!(
            ferrari,
            instances.instance("ferrari", &source, Handle::default(), &mut materials)
        );
        let painted = materials.get(&ferrari).unwrap();
        assert_eq!(painted.base_color, Color::WHITE);
        assert_eq!(painted.normal_map_texture, Some(normal));
        assert_eq!(painted.perceptual_roughness, original.perceptual_roughness);
        assert_eq!(painted.base_color_channel, UvChannel::Uv0);
        assert!(materials.get(&source).unwrap().base_color_texture.is_none());
        assert_eq!(
            materials.get(&source).unwrap().base_color,
            original.base_color
        );
    }

    /// Inspect real compound instances, UV artwork and the BC7 path in all LODs.
    #[test]
    #[ignore = "requires a Vulkan display and F1_TEST_ASSETS"]
    fn rendered_tyre_compounds() {
        let root = std::env::var("F1_TEST_ASSETS").expect("set F1_TEST_ASSETS");
        let options = crate::options::Options::parse(
            [
                "--assets",
                &root,
                "--car",
                "gen22",
                "--team",
                "mclaren",
                "--graphics",
                "ultra",
                "--size",
                "1280x720",
                "--mute",
                "--frames",
                "10000",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .unwrap();
        let mut app = crate::build_app(options).unwrap();
        {
            let mut session = app.world_mut().resource_mut::<crate::Session>();
            for (index, entrant) in session.race.entrants.iter_mut().enumerate() {
                entrant.car.setup.compound = racecraft_sim::Compound::ALL[index % 3];
            }
        }
        app.add_systems(Update, tyre_review.after(crate::camera::update));
        assert!(app.run().is_success());
    }

    #[allow(clippy::too_many_arguments)]
    fn tyre_review(
        mut commands: Commands,
        mut session: ResMut<crate::Session>,
        mut cameras: Query<(&mut Transform, &mut Projection), With<bevy_egui::PrimaryEguiContext>>,
        surfaces: Query<
            (
                &CarSurface,
                &GltfMaterialName,
                &MeshMaterial3d<StandardMaterial>,
            ),
            With<SurfaceReady>,
        >,
        materials: Res<Assets<StandardMaterial>>,
        images: Res<Assets<Image>>,
        device: Res<bevy::render::renderer::RenderDevice>,
        server: Res<AssetServer>,
        mut frame: Local<u32>,
        mut captured: Local<usize>,
        mut exit: MessageWriter<AppExit>,
    ) {
        if !session.ready {
            return;
        }
        session.paused = true;
        *frame += 1;
        if *frame == 40 {
            let mut checked = std::collections::BTreeSet::new();
            for (surface, name, handle) in &surfaces {
                if name.0 != "tyre" {
                    continue;
                }
                let material = materials.get(&handle.0).unwrap();
                assert_eq!(material.perceptual_roughness, 0.85);
                assert_eq!(material.reflectance, 0.5);
                assert_eq!(material.metallic, 0.);
                assert_eq!(
                    material.base_color,
                    Color::linear_rgb(TYRE_TINT, TYRE_TINT, TYRE_TINT)
                );
                let texture = material.base_color_texture.as_ref().unwrap();
                let image = images.get(texture).unwrap();
                assert!(image.texture_descriptor.mip_level_count > 1);
                if device
                    .features()
                    .contains(bevy::render::render_resource::WgpuFeatures::TEXTURE_COMPRESSION_BC)
                {
                    assert_eq!(
                        image.texture_descriptor.format,
                        bevy::render::render_resource::TextureFormat::Bc7RgbaUnormSrgb,
                    );
                }
                assert_eq!(
                    server.get_path(texture.id()).unwrap().path(),
                    std::path::Path::new(&tyre_texture(surface.lod, surface.compound))
                );
                checked.insert((surface.lod, surface.compound.label()));
            }
            assert_eq!(
                checked.len(),
                7,
                "player plus three compounds in both opponent LODs"
            );
            info!("Tyre compound instances verified: {checked:?}");
        }
        let stage = (*frame / 60).min(5);
        let entrant = [session.player, 0, 1, 2, 0, 2][stage as usize];
        let pose = crate::pose::Pose::of(&session.race.entrants[entrant].car);
        let origin = pose.origin.as_vec3();
        let rotation = Quat::from_rotation_y(pose.yaw as f32);
        let offset = if stage < 4 {
            Vec3::new(3., 1., 1.4)
        } else {
            Vec3::new(-15., 40., 28.)
        };
        for (mut camera, mut projection) in &mut cameras {
            if let Projection::Perspective(lens) = &mut *projection {
                lens.fov = if stage < 4 { 60_f32 } else { 7_f32 }.to_radians();
            }
            *camera = Transform::from_translation(origin + rotation * offset)
                .looking_at(origin + rotation * Vec3::new(0., 0.4, 1.3), Vec3::Y);
        }
        if *frame % 60 == 50 && *frame < 360 {
            use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
            let directory = std::path::PathBuf::from(
                std::env::var("F1_TEST_OUTPUT").expect("set F1_TEST_OUTPUT"),
            );
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join(format!("tyre-review-{stage}.png"));
            commands.spawn(Screenshot::primary_window()).observe(
                move |event: On<ScreenshotCaptured>| {
                    event
                        .image
                        .clone()
                        .try_into_dynamic()
                        .unwrap()
                        .to_rgb8()
                        .save(&path)
                        .unwrap();
                },
            );
            *captured += 1;
        }
        if *frame == 360 {
            assert_eq!(*captured, 6);
            exit.write(AppExit::Success);
        }
    }

    #[test]
    fn compound_textures_follow_each_entrant_at_every_lod() {
        for compound in racecraft_sim::Compound::ALL {
            let name = compound.label().to_ascii_lowercase();
            for lod in [None, Some("lod1"), Some("lod2")] {
                let path = tyre_texture(lod, compound);
                assert!(path.ends_with(&format!("/{name}.png")));
                assert_eq!(path.contains("lod1/"), lod == Some("lod1"));
                assert_eq!(path.contains("lod2/"), lod == Some("lod2"));
            }
        }
    }

    #[test]
    fn source_pbr_survives_runtime_preparation() {
        for name in [
            "source_BMD_Brass",
            "source_Translucent Plastic_red",
            "wheel_legends",
        ] {
            let mut material = StandardMaterial {
                base_color: Color::srgb(0.1, 0.02, 0.01),
                metallic: 1.,
                perceptual_roughness: 0.18,
                clearcoat: 0.25,
                emissive: LinearRgba::rgb(0.12, 0.01, 0.01),
                ..default()
            };
            let expected = material.clone();
            calibrate(name, &mut material);
            assert_eq!(material.base_color, expected.base_color);
            assert_eq!(material.metallic, expected.metallic);
            assert_eq!(material.perceptual_roughness, expected.perceptual_roughness);
            assert_eq!(material.clearcoat, expected.clearcoat);
            assert_eq!(material.emissive, expected.emissive);
        }
    }

    #[test]
    fn stock_gen22_paint_has_no_artwork_and_preserves_surface_detail() {
        let mut paint = StandardMaterial {
            base_color: Color::srgb(0.02, 0.01, 0.005),
            clearcoat: 0.16,
            reflectance: 0.5,
            perceptual_roughness: 0.3,
            ..default()
        };
        calibrate("source_paint_main_body_stickers_lod.001", &mut paint);
        assert_eq!(paint.base_color, Color::linear_rgb(0.32, 0.34, 0.36));
        assert!(paint.base_color_texture.is_none());
        assert_eq!(paint.perceptual_roughness, 0.3);
        assert_eq!(paint.clearcoat, 0.12);
        assert_eq!(paint.reflectance, 0.4);
    }

    #[test]
    fn paint_is_dielectric_and_rubber_has_a_dielectric_sheen() {
        let mut paint = StandardMaterial {
            metallic: 0.8,
            perceptual_roughness: 0.,
            base_color: Color::srgb(0.05, 0.08, 0.2),
            ..default()
        };
        let color = paint.base_color;
        calibrate("body", &mut paint);
        assert_eq!(paint.base_color, color);
        assert_eq!(paint.metallic, 0.);
        assert!(paint.perceptual_roughness >= 0.3);
        for name in ["tyre", "tread", "sidewall.007"] {
            let mut tyre = StandardMaterial {
                base_color: Color::linear_rgb(0.04, 0.04, 0.04),
                ..default()
            };
            let rubber = tyre.base_color;
            calibrate(name, &mut tyre);
            assert_eq!(tyre.base_color, rubber);
            assert_eq!(tyre.perceptual_roughness, 0.85);
            assert_eq!(tyre.reflectance, 0.5);
            assert_eq!(tyre.metallic, 0.);
        }
    }
}
