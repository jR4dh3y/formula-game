//! glTF's embedded PNGs have no mip chain. Build it once before rendering the scene.

use bevy::{
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::{TextureDimension, TextureFormat},
};

pub fn prepare(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        // Renderer lookup tables are data, not color textures. Never filter them.
        if !server
            .get_path(*id)
            .is_some_and(|path| path.path().extension().is_some_and(|ext| ext == "glb"))
        {
            continue;
        }
        let Some(mut image) = images.get_mut(*id) else {
            continue;
        };
        if generate_mips(&mut image) {
            let mut sampler = match &image.sampler {
                ImageSampler::Descriptor(sampler) => sampler.clone(),
                ImageSampler::Default => ImageSamplerDescriptor::linear(),
            };
            sampler.set_anisotropic_filter(16);
            sampler.mipmap_filter = ImageFilterMode::Linear;
            image.sampler = ImageSampler::Descriptor(sampler);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{asset::RenderAssetUsages, render::render_resource::Extent3d};

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
