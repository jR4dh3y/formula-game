//! Bring imported car surfaces into the same daylight response as the track.

use bevy::{gltf::GltfMaterialName, prelude::*};
use std::collections::HashSet;

#[derive(Component)]
pub(crate) struct CarSurface;

#[derive(Component)]
pub(crate) struct SurfaceReady;

#[derive(Component)]
pub(crate) struct WheelSurface;

type PendingSurfaces<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static GltfMaterialName,
        &'static MeshMaterial3d<StandardMaterial>,
        Option<&'static WheelSurface>,
    ),
    (With<CarSurface>, Without<SurfaceReady>),
>;

pub fn prepare(
    mut commands: Commands,
    surfaces: PendingSurfaces,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut prepared: Local<HashSet<AssetId<StandardMaterial>>>,
    mut wheel_materials: Local<
        std::collections::HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    >,
) {
    for (entity, name, handle, wheel) in &surfaces {
        if wheel.is_some() && matches!(name.0.as_str(), "carbon" | "carbon_gloss" | "rubber") {
            let replacement = if let Some(cached) = wheel_materials.get(&handle.0.id()) {
                cached.clone()
            } else {
                let Some(mut material) = materials.get(&handle.0).cloned() else {
                    continue;
                };
                calibrate(&name.0, &mut material);
                // The imported near-black values lose all control detail in cockpit shadow.
                // Keep this override local to the wheel, since the floor shares its material.
                material.base_color = if name.0 == "rubber" {
                    Color::srgb(0.16, 0.17, 0.18)
                } else {
                    Color::srgb(0.28, 0.30, 0.32)
                };
                let replacement = materials.add(material);
                wheel_materials.insert(handle.0.id(), replacement.clone());
                replacement
            };
            commands
                .entity(entity)
                .insert((MeshMaterial3d(replacement), SurfaceReady));
            continue;
        }
        let Some(mut material) = materials.get_mut(&handle.0) else {
            continue;
        };
        if prepared.insert(handle.0.id()) {
            calibrate(&name.0, &mut material);
        }
        commands.entity(entity).insert(SurfaceReady);
    }
}

fn calibrate(name: &str, material: &mut StandardMaterial) {
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
        material.perceptual_roughness = 0.88;
        material.reflectance = 0.22;
        material.clearcoat = 0.;
    } else if name == "paint" {
        material.metallic = 0.;
        // The shared atlas includes exposed carbon as well as coloured paint.
        material.perceptual_roughness = 0.48;
        material.reflectance = 0.22;
        material.clearcoat = 0.;
        material.clearcoat_perceptual_roughness = 0.2;
    } else if matches!(name.as_str(), "suit" | "helmet_paint" | "rubber" | "brake") {
        material.metallic = 0.;
        material.perceptual_roughness = if name == "helmet_paint" { 0.35 } else { 0.8 };
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
        // The atlas also includes exposed carbon, so a uniform clearcoat would
        // turn the black floor and wings into bright sky-colored reflections.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_paint_atlas_keeps_carbon_free_of_uniform_clearcoat() {
        let mut material = StandardMaterial {
            clearcoat: 1.,
            ..default()
        };
        let color = material.base_color;
        calibrate("paint", &mut material);
        assert_eq!(material.clearcoat, 0.);
        assert_eq!(material.base_color, color);
        assert!(material.reflectance < 0.3);
    }

    #[test]
    fn paint_is_dielectric_and_rubber_stays_matte_without_replacing_livery() {
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
        let mut tyre = StandardMaterial::default();
        calibrate("sidewall.007", &mut tyre);
        assert!(tyre.perceptual_roughness > 0.8 && tyre.reflectance < 0.3);
    }
}
