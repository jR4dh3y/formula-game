//! A procedural daylight sky panorama (equirectangular, linear HDR).

use dhool_pro::math::Color;
use dhool_pro::{EnvironmentEncoding, EnvironmentMap, TextureImage};

pub fn daylight(sun_direction: [f32; 3]) -> EnvironmentMap {
    const WIDTH: u32 = 512;
    const HEIGHT: u32 = 256;
    let to_sun = {
        let [x, y, z] = sun_direction;
        let length = (x * x + y * y + z * z).sqrt().max(1e-6);
        [-x / length, -y / length, -z / length]
    };
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    for row in 0..HEIGHT {
        let elevation =
            std::f32::consts::FRAC_PI_2 - (row as f32 + 0.5) / HEIGHT as f32 * std::f32::consts::PI;
        for column in 0..WIDTH {
            // Matches EnvironmentMap::sample_radiance's u = 0.5 + atan2(z, x) / 2π.
            let azimuth = ((column as f32 + 0.5) / WIDTH as f32 - 0.5) * std::f32::consts::TAU;
            let direction = [
                elevation.cos() * azimuth.cos(),
                elevation.sin(),
                elevation.cos() * azimuth.sin(),
            ];
            let up = direction[1];
            let color = if up >= 0.0 {
                let zenith = [0.16, 0.34, 0.78];
                let horizon = [0.62, 0.74, 0.88];
                let t = up.powf(0.45);
                let mut sky = [0.0; 3];
                for channel in 0..3 {
                    sky[channel] = horizon[channel] + (zenith[channel] - horizon[channel]) * t;
                }
                let cos_sun =
                    direction[0] * to_sun[0] + direction[1] * to_sun[1] + direction[2] * to_sun[2];
                let glow = cos_sun.max(0.0).powf(64.0) * 2.5 + cos_sun.max(0.0).powf(8.0) * 0.25;
                let disc = if cos_sun > 0.9995 { 40.0 } else { 0.0 };
                [
                    sky[0] + glow + disc,
                    sky[1] + glow * 0.92 + disc,
                    sky[2] + glow * 0.75 + disc,
                ]
            } else {
                // Dark green-grey ground below the horizon.
                let t = (-up).powf(0.5);
                [0.20 - 0.08 * t, 0.24 - 0.09 * t, 0.18 - 0.08 * t]
            };
            pixels.push(Color::new(color[0], color[1], color[2], 1.0));
        }
    }
    EnvironmentMap::from_image(
        TextureImage {
            width: WIDTH,
            height: HEIGHT,
            pixels: pixels.into(),
        },
        EnvironmentEncoding::LinearHdr,
        0.0,
        0.0,
    )
}
