//! glTF's embedded PNGs have no mip chain. Build it once before rendering the scene.

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::{
        render_resource::{TextureDimension, TextureFormat, WgpuFeatures},
        renderer::RenderDevice,
    },
};

/// Top mip levels to drop from glTF textures as they load (video texture detail).
#[derive(Resource, Default)]
pub struct TextureDetail(pub u32);

/// Bevy 0.20 ignores normalTexture.scale; retain it while its glTF loader has the metadata.
#[derive(Resource, Default, Clone)]
pub struct NormalScales(
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<AssetId<Image>, f32>>>,
);

#[derive(Clone, Default)]
struct NormalScaleLoader {
    pending: NormalScales,
    scales: std::collections::HashMap<usize, f32>,
}

pub fn register_normal_scales(app: &mut App) {
    use bevy::gltf::extensions::GltfExtensionHandlers;
    let pending = NormalScales::default();
    app.world_mut()
        .resource_mut::<GltfExtensionHandlers>()
        .0
        .write_blocking()
        .push(Box::new(NormalScaleLoader {
            pending: pending.clone(),
            ..default()
        }));
    app.insert_resource(pending);
}

impl bevy::gltf::extensions::GltfExtensionHandler for NormalScaleLoader {
    fn dyn_clone(&self) -> Box<dyn bevy::gltf::extensions::ErasedGltfExtensionHandler> {
        Box::new(self.clone())
    }

    fn on_root(
        &mut self,
        context: &mut bevy::asset::LoadContext<'_>,
        gltf: &bevy::gltf::gltf::Gltf,
        _: &bevy::gltf::GltfLoaderSettings,
    ) {
        self.scales.clear();
        if !context.path().path().starts_with("cars/gen22") {
            return;
        }
        for material in gltf.materials() {
            if let Some(normal) = material.normal_texture() {
                if normal.scale() != 1. {
                    self.scales.insert(normal.texture().index(), normal.scale());
                }
            }
        }
    }

    fn on_texture(&mut self, texture: &bevy::gltf::gltf::Texture, image: Handle<Image>) {
        if let Some(scale) = self.scales.get(&texture.index()) {
            self.pending.0.lock().unwrap().insert(image.id(), *scale);
        }
    }
}

fn scale_normals(image: &mut Image, scale: f32) {
    if image.texture_descriptor.format != TextureFormat::Rgba8Unorm {
        return;
    }
    let Some(data) = image.data.as_mut() else {
        return;
    };
    for pixel in data.chunks_exact_mut(4) {
        let normal = Vec3::new(
            (pixel[0] as f32 / 255. * 2. - 1.) * scale,
            (pixel[1] as f32 / 255. * 2. - 1.) * scale,
            pixel[2] as f32 / 255. * 2. - 1.,
        )
        .normalize_or(Vec3::Z);
        for (channel, value) in pixel[..3].iter_mut().zip(normal.to_array()) {
            *channel = ((value * 0.5 + 0.5) * 255.).round().clamp(0., 255.) as u8;
        }
    }
}

pub fn prepare(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
    detail: Option<Res<TextureDetail>>,
    device: Option<Res<RenderDevice>>,
    normal_scales: Res<NormalScales>,
) {
    let dropped = detail.map_or(0, |detail| detail.0);
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        // Renderer lookup tables are data, not color textures. Never filter them.
        if !server.get_path(*id).is_some_and(|path| {
            let path = path.path();
            path.extension().is_some_and(|ext| ext == "glb")
                || path.starts_with("cars/gen22/liveries")
                || path.starts_with("cars/gen22/tyres")
        }) {
            continue;
        }
        let Some(image) = images.get_mut_untracked(*id) else {
            continue;
        };
        if let Some(scale) = normal_scales.0.lock().unwrap().remove(id) {
            scale_normals(image, scale);
        }
        if generate_mips(image) {
            drop_top_mips(image, dropped);
            let mut sampler = match &image.sampler {
                ImageSampler::Descriptor(sampler) => sampler.clone(),
                ImageSampler::Default => ImageSamplerDescriptor::linear(),
            };
            sampler.set_anisotropic_filter(16);
            sampler.mipmap_filter = ImageFilterMode::Linear;
            image.sampler = ImageSampler::Descriptor(sampler);
            if device.as_ref().is_some_and(|device| {
                device
                    .features()
                    .contains(WgpuFeatures::TEXTURE_COMPRESSION_BC)
            }) {
                compress_bc7(image);
            }
            // Immutable imported textures do not need a second copy in the main world.
            image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        }
    }
}

/// Encode the complete mip chain without changing its resolution or colour space.
fn compress_bc7(image: &mut Image) {
    let desc = &image.texture_descriptor;
    if !desc.size.width.is_multiple_of(4) || !desc.size.height.is_multiple_of(4) {
        return;
    }
    let Some(data) = image.data.as_ref() else {
        return;
    };
    let (mut width, mut height) = (desc.size.width, desc.size.height);
    let output_bytes: usize = (0..desc.mip_level_count)
        .map(|level| {
            (width.checked_shr(level).unwrap_or(0).max(1).div_ceil(4)
                * height.checked_shr(level).unwrap_or(0).max(1).div_ceil(4)
                * 16) as usize
        })
        .sum();
    let cache = compressed_cache_path(image);
    if let Some(blocks) = cache
        .as_ref()
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| bytes.len() == output_bytes)
    {
        set_bc7(image, blocks);
        return;
    }
    let mut offset = 0;
    let mut blocks = Vec::new();
    for _ in 0..desc.mip_level_count {
        let bytes = (width * height * 4) as usize;
        let source = &data[offset..offset + bytes];
        let (padded_width, padded_height) = (width.div_ceil(4) * 4, height.div_ceil(4) * 4);
        // Sub-block tail mips still occupy one complete 4x4 block.
        let padded;
        let pixels = if padded_width != width || padded_height != height {
            padded = (0..padded_height)
                .flat_map(|y| {
                    (0..padded_width).flat_map(move |x| {
                        let pixel = ((y.min(height - 1) * width + x.min(width - 1)) * 4) as usize;
                        source[pixel..pixel + 4].iter().copied()
                    })
                })
                .collect::<Vec<_>>();
            &padded
        } else {
            source
        };
        let surface = intel_tex_2::RgbaSurface {
            data: pixels,
            width: padded_width,
            height: padded_height,
            stride: padded_width * 4,
        };
        blocks.extend(intel_tex_2::bc7::compress_blocks(
            &intel_tex_2::bc7::alpha_ultra_fast_settings(),
            &surface,
        ));
        offset += bytes;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    if let Some(path) = cache {
        if let Some(parent) = path.parent() {
            let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
            if std::fs::create_dir_all(parent).is_ok()
                && std::fs::write(&temporary, &blocks).is_ok()
            {
                let _ = std::fs::rename(&temporary, path);
            }
        }
    }
    set_bc7(image, blocks);
}

fn set_bc7(image: &mut Image, blocks: Vec<u8>) {
    image.data = Some(blocks);
    image.texture_descriptor.format = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => TextureFormat::Bc7RgbaUnormSrgb,
        _ => TextureFormat::Bc7RgbaUnorm,
    };
}

/// Content addressing invalidates cached blocks when pixels, mips or colour space change.
fn compressed_cache_path(image: &Image) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".cache"))
        })?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let desc = &image.texture_descriptor;
    (
        desc.size.width,
        desc.size.height,
        desc.mip_level_count,
        desc.format,
    )
        .hash(&mut hash);
    image.data.as_ref()?.hash(&mut hash);
    Some(
        root.join("f1-racecraft/bc7-v1")
            .join(format!("{:016x}.bin", hash.finish())),
    )
}

fn generate_mips(image: &mut Image) -> bool {
    let desc = &image.texture_descriptor;
    if desc.dimension != TextureDimension::D2
        || desc.size.depth_or_array_layers != 1
        || desc.mip_level_count != 1
        || !matches!(
            desc.format,
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
    {
        return false;
    }
    let Some(data) = image.data.as_mut() else {
        return false;
    };
    let (mut width, mut height) = (desc.size.width as usize, desc.size.height as usize);
    if data.len() != width * height * 4 || width.max(height) <= 1 {
        return false;
    }
    let srgb = desc.format == TextureFormat::Rgba8UnormSrgb;
    let decode: [f32; 256] = std::array::from_fn(|i| {
        let x = i as f32 / 255.;
        if srgb && x > 0.04045 {
            ((x + 0.055) / 1.055).powf(2.4)
        } else if srgb {
            x / 12.92
        } else {
            x
        }
    });
    let encode = |x: f32| {
        let x = if srgb && x > 0.0031308 {
            1.055 * x.powf(1. / 2.4) - 0.055
        } else if srgb {
            x * 12.92
        } else {
            x
        };
        (x * 255.).round().clamp(0., 255.) as u8
    };
    let mut offset = 0;
    let mut levels = 1;
    while width > 1 || height > 1 {
        let (next_width, next_height) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = vec![0; next_width * next_height * 4];
        for y in 0..next_height {
            for x in 0..next_width {
                let mut sum = [0.; 4];
                let mut count = 0.;
                // Integer areas include the last texel for odd-sized source images.
                for sy in y * height / next_height..(y + 1) * height / next_height {
                    for sx in x * width / next_width..(x + 1) * width / next_width {
                        let p = offset + (sy * width + sx) * 4;
                        let alpha = data[p + 3] as f32 / 255.;
                        for c in 0..3 {
                            sum[c] += decode[data[p + c] as usize] * alpha;
                        }
                        sum[3] += alpha;
                        count += 1.;
                    }
                }
                let out = (y * next_width + x) * 4;
                if sum[3] > 0. {
                    for c in 0..3 {
                        next[out + c] = encode(sum[c] / sum[3]);
                    }
                }
                next[out + 3] = (sum[3] / count * 255.).round() as u8;
            }
        }
        offset = data.len();
        data.extend(next);
        width = next_width;
        height = next_height;
        levels += 1;
    }
    image.texture_descriptor.mip_level_count = levels;
    true
}

/// Discard up to `count` of the largest mip levels, keeping at least 256 px.
fn drop_top_mips(image: &mut Image, count: u32) {
    let desc = &image.texture_descriptor;
    let (mut width, mut height) = (desc.size.width as usize, desc.size.height as usize);
    let mut levels = desc.mip_level_count;
    let mut offset = 0;
    let mut dropped = 0;
    while dropped < count && levels > 1 && width.max(height) > 256 {
        offset += width * height * 4;
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        levels -= 1;
        dropped += 1;
    }
    if dropped == 0 {
        return;
    }
    let Some(data) = image.data.as_mut() else {
        return;
    };
    data.drain(..offset);
    let desc = &mut image.texture_descriptor;
    desc.size.width = width as u32;
    desc.size.height = height as u32;
    desc.mip_level_count = levels;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::RenderAssetUsages, render::render_resource::Extent3d};

    #[test]
    fn normal_strength_changes_tangent_slope_without_changing_colour_or_alpha() {
        let mut image = Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![204, 128, 230, 77],
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        scale_normals(&mut image, 0.16);
        let pixel = image.data.as_ref().unwrap();
        let x = pixel[0] as f32 / 255. * 2. - 1.;
        let z = pixel[2] as f32 / 255. * 2. - 1.;
        let original_slope = (204. / 255. * 2. - 1.) / (230. / 255. * 2. - 1.);
        assert!((x / z - original_slope * 0.16).abs() < 0.01);
        assert_eq!(pixel[3], 77);
        image.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
        let colour = image.data.clone();
        scale_normals(&mut image, 0.16);
        assert_eq!(image.data, colour);
    }

    #[test]
    fn bc7_preserves_resolution_colour_space_and_the_complete_mip_tail() {
        for (format, compressed) in [
            (
                TextureFormat::Rgba8UnormSrgb,
                TextureFormat::Bc7RgbaUnormSrgb,
            ),
            (TextureFormat::Rgba8Unorm, TextureFormat::Bc7RgbaUnorm),
        ] {
            let mut image = Image::new(
                Extent3d {
                    width: 16,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                vec![128; 16 * 8 * 4],
                format,
                RenderAssetUsages::default(),
            );
            assert!(generate_mips(&mut image));
            compress_bc7(&mut image);
            assert_eq!(image.texture_descriptor.format, compressed);
            assert_eq!(image.texture_descriptor.size.width, 16);
            assert_eq!(image.texture_descriptor.size.height, 8);
            assert_eq!(image.texture_descriptor.mip_level_count, 5);
            // 16x8, 8x4, 4x2, 2x1 and 1x1, rounded to complete blocks.
            assert_eq!(image.data.as_ref().unwrap().len(), (8 + 2 + 1 + 1 + 1) * 16);
        }
    }

    #[test]
    fn dropping_top_mips_keeps_the_smaller_levels_and_a_256_pixel_floor() {
        let size = 1024;
        let mut image = Image::new(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![200; (size * size * 4) as usize],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut image));
        let levels = image.texture_descriptor.mip_level_count;
        let level_bytes = |side: usize| side * side * 4;
        drop_top_mips(&mut image, 1);
        assert_eq!(image.texture_descriptor.size.width, 512);
        assert_eq!(image.texture_descriptor.mip_level_count, levels - 1);
        let remaining: usize = (0..levels - 1).map(|i| level_bytes(512 >> i)).sum();
        assert_eq!(image.data.as_ref().unwrap().len(), remaining);
        drop_top_mips(&mut image, 4);
        assert_eq!(image.texture_descriptor.size.width, 256);
        assert_eq!(image.texture_descriptor.mip_level_count, levels - 2);
    }

    #[test]
    fn mip_chain_filters_color_in_linear_light_and_reaches_one_pixel() {
        let mut image = Image::new(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut image));
        assert_eq!(image.texture_descriptor.mip_level_count, 2);
        assert_eq!(&image.data.as_ref().unwrap()[16..], &[188, 188, 188, 255]);
        assert!(!generate_mips(&mut image));
    }

    #[test]
    fn transparent_texels_do_not_bleed_color_into_fence_edges() {
        let mut image = Image::new(
            Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![255, 0, 0, 0, 0, 255, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        assert!(generate_mips(&mut image));
        assert_eq!(&image.data.as_ref().unwrap()[8..], &[0, 255, 0, 128]);
    }
}
