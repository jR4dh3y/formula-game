use crate::asset::{
    MaterialDesc, MeshDesc, ShadowCasterClass, TextureDesc, TextureFormat, TextureRole,
};
use crate::cloth::ClothDefinition;
use crate::math::{Color, Mat4, Vec2, Vec3, Vec4};
use std::f32::consts::{PI, TAU};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialClass {
    Default,
    Concrete,
    Foliage,
    Skin,
    Hair,
    Glass,
    Metal,
    Emissive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterialFlags {
    pub glass: bool,
    pub emissive: bool,
    pub receives_contact_shadows: bool,
    pub alpha_tested: bool,
    pub uses_bent_normals: bool,
    pub responsive_aa: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub class: MaterialClass,
    pub base_color: Color,
    pub base_color_texture: Option<Arc<TextureImage>>,
    pub normal_texture: Option<Arc<TextureImage>>,
    pub pbr_texture: Option<Arc<TextureImage>>,
    pub roughness: f32,
    pub metallic: f32,
    pub specular_intensity: f32,
    pub cavity: f32,
    pub material_id: u8,
    pub texture_sample_count: usize,
    pub flags: MaterialFlags,
    pub emission: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub pixels: TexturePixels,
}

/// Texel storage. Display-referred images stay 8-bit; HDR panoramas keep floats.
#[derive(Clone, Debug, PartialEq)]
pub enum TexturePixels {
    Rgba8(Vec<[u8; 4]>),
    Float(Vec<Color>),
}

impl TexturePixels {
    pub fn len(&self) -> usize {
        match self {
            Self::Rgba8(pixels) => pixels.len(),
            Self::Float(pixels) => pixels.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<Color> {
        match self {
            Self::Rgba8(pixels) => pixels.get(index).map(|&pixel| Color::from_rgba8(pixel)),
            Self::Float(pixels) => pixels.get(index).copied(),
        }
    }

    pub fn color(&self, index: usize) -> Color {
        match self {
            Self::Rgba8(pixels) => Color::from_rgba8(pixels[index]),
            Self::Float(pixels) => pixels[index],
        }
    }

    pub fn to_rgba8(&self) -> Vec<[u8; 4]> {
        match self {
            Self::Rgba8(pixels) => pixels.clone(),
            Self::Float(pixels) => pixels.iter().map(|color| color.to_rgba8()).collect(),
        }
    }

    pub fn alpha(&self) -> Vec<f32> {
        match self {
            Self::Rgba8(pixels) => pixels.iter().map(|pixel| pixel[3] as f32 / 255.0).collect(),
            Self::Float(pixels) => pixels.iter().map(|color| color.a).collect(),
        }
    }
}

impl From<Vec<Color>> for TexturePixels {
    fn from(pixels: Vec<Color>) -> Self {
        Self::Float(pixels)
    }
}

impl From<Vec<[u8; 4]>> for TexturePixels {
    fn from(pixels: Vec<[u8; 4]>) -> Self {
        Self::Rgba8(pixels)
    }
}

impl TextureImage {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::from_path_with_max_dimension(path, 0)
    }

    pub fn from_path_with_max_dimension(
        path: impl AsRef<Path>,
        max_dimension: u32,
    ) -> Result<Self, String> {
        let image = image::open(path.as_ref()).map_err(|error| error.to_string())?;
        Self::from_dynamic_image(image, max_dimension)
    }

    pub fn from_encoded_bytes_with_max_dimension(
        bytes: &[u8],
        max_dimension: u32,
    ) -> Result<Self, String> {
        let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
        Self::from_dynamic_image(image, max_dimension)
    }

    fn from_dynamic_image(
        mut image: image::DynamicImage,
        max_dimension: u32,
    ) -> Result<Self, String> {
        if max_dimension > 0 {
            let largest = image.width().max(image.height());
            if largest > max_dimension {
                let scale = max_dimension as f32 / largest as f32;
                let width = ((image.width() as f32 * scale).round() as u32).max(1);
                let height = ((image.height() as f32 * scale).round() as u32).max(1);
                image = image.resize(width, height, image::imageops::FilterType::Triangle);
            }
        }
        match image {
            image::DynamicImage::ImageRgb32F(image) => Ok(Self {
                width: image.width(),
                height: image.height(),
                pixels: TexturePixels::Float(
                    image
                        .pixels()
                        .map(|pixel| Color::new(pixel[0], pixel[1], pixel[2], 1.0))
                        .collect(),
                ),
            }),
            image::DynamicImage::ImageRgba32F(image) => Ok(Self {
                width: image.width(),
                height: image.height(),
                pixels: TexturePixels::Float(
                    image
                        .pixels()
                        .map(|pixel| Color::new(pixel[0], pixel[1], pixel[2], pixel[3]))
                        .collect(),
                ),
            }),
            image => {
                let rgba = image.to_rgba8();
                Ok(Self {
                    width: rgba.width(),
                    height: rgba.height(),
                    pixels: TexturePixels::Rgba8(rgba.pixels().map(|pixel| pixel.0).collect()),
                })
            }
        }
    }

    pub fn sample_repeat(&self, uv: Vec2) -> Color {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return Color::WHITE;
        }
        let u = uv.x.rem_euclid(1.0);
        let v = uv.y.rem_euclid(1.0);
        let x = (u * (self.width - 1) as f32).round() as u32;
        let y = ((1.0 - v) * (self.height - 1) as f32).round() as u32;
        self.pixels.color((y * self.width + x) as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentEncoding {
    LinearHdr,
    Srgb,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentMap {
    pub panorama: Arc<TextureImage>,
    pub encoding: EnvironmentEncoding,
    linear_tint: Vec3,
    exposure_ev: f32,
    yaw_radians: f32,
    irradiance_sh: [[f32; 3]; 9],
}

impl EnvironmentMap {
    pub fn from_image(
        panorama: TextureImage,
        encoding: EnvironmentEncoding,
        exposure_ev: f32,
        yaw_radians: f32,
    ) -> Self {
        Self::new(panorama, encoding, exposure_ev, yaw_radians)
    }

    pub fn from_hdr_path(
        path: impl AsRef<Path>,
        exposure_ev: f32,
        yaw_radians: f32,
    ) -> Result<Self, String> {
        Ok(Self::new(
            TextureImage::from_path(path)?,
            EnvironmentEncoding::LinearHdr,
            exposure_ev,
            yaw_radians,
        ))
    }

    pub fn from_srgb_path(
        path: impl AsRef<Path>,
        exposure_ev: f32,
        yaw_radians: f32,
    ) -> Result<Self, String> {
        Ok(Self::new(
            TextureImage::from_path(path)?,
            EnvironmentEncoding::Srgb,
            exposure_ev,
            yaw_radians,
        ))
    }

    pub fn solid(color: Color) -> Self {
        Self::new(
            TextureImage {
                width: 2,
                height: 1,
                pixels: TexturePixels::Float(vec![color; 2]),
            },
            EnvironmentEncoding::Srgb,
            0.0,
            0.0,
        )
    }

    pub fn from_hdr_bytes(
        bytes: &[u8],
        exposure_ev: f32,
        yaw_radians: f32,
    ) -> Result<Self, String> {
        Ok(Self::new(
            TextureImage::from_encoded_bytes_with_max_dimension(bytes, 0)?,
            EnvironmentEncoding::LinearHdr,
            exposure_ev,
            yaw_radians,
        ))
    }

    fn new(
        panorama: TextureImage,
        encoding: EnvironmentEncoding,
        exposure_ev: f32,
        yaw_radians: f32,
    ) -> Self {
        let mut environment = Self {
            panorama: Arc::new(panorama),
            encoding,
            linear_tint: Vec3::ONE,
            exposure_ev,
            yaw_radians,
            irradiance_sh: [[0.0; 3]; 9],
        };
        environment.irradiance_sh = environment.build_irradiance_sh();
        environment
    }

    pub fn with_linear_tint(mut self, tint: Vec3) -> Self {
        self.linear_tint = Vec3::new(tint.x.max(0.0), tint.y.max(0.0), tint.z.max(0.0));
        self.irradiance_sh = self.build_irradiance_sh();
        self
    }

    pub fn with_exposure_and_yaw(mut self, exposure_ev: f32, yaw_radians: f32) -> Self {
        self.exposure_ev = exposure_ev;
        self.yaw_radians = yaw_radians;
        self.irradiance_sh = self.build_irradiance_sh();
        self
    }

    pub fn sample_radiance(&self, direction: Vec3) -> Vec3 {
        let direction = direction.normalize_or_zero();
        if direction == Vec3::ZERO {
            return Vec3::ZERO;
        }
        let uv = Vec2::new(
            (0.5 + (direction.z.atan2(direction.x) + self.yaw_radians) / TAU).fract(),
            (0.5 - direction.y.clamp(-1.0, 1.0).asin() / PI).clamp(0.0, 1.0),
        );
        let sample = self.sample_panorama(uv);
        let radiance = self.decode_radiance(sample);
        radiance * self.linear_tint * self.exposure_ev.exp2()
    }

    pub const fn exposure_ev(&self) -> f32 {
        self.exposure_ev
    }

    pub const fn yaw_radians(&self) -> f32 {
        self.yaw_radians
    }

    pub const fn irradiance_sh(&self) -> &[[f32; 3]; 9] {
        &self.irradiance_sh
    }

    pub fn linear_texel(&self, index: usize) -> Vec3 {
        self.panorama
            .pixels
            .get(index)
            .map(|sample| self.decode_radiance(sample) * self.linear_tint)
            .unwrap_or(Vec3::ZERO)
    }

    pub fn diffuse_irradiance(&self, normal: Vec3) -> Vec3 {
        let basis = sh_basis(normal.normalize_or_zero());
        let mut irradiance = Vec3::ZERO;
        for (coefficient, basis) in self.irradiance_sh.iter().zip(basis) {
            irradiance += Vec3::new(coefficient[0], coefficient[1], coefficient[2]) * basis;
        }
        Vec3::new(
            irradiance.x.max(0.0),
            irradiance.y.max(0.0),
            irradiance.z.max(0.0),
        )
    }

    fn sample_panorama(&self, uv: Vec2) -> Color {
        if self.panorama.width == 0 || self.panorama.height == 0 || self.panorama.pixels.is_empty()
        {
            return Color::BLACK;
        }
        let width = self.panorama.width as i32;
        let height = self.panorama.height as i32;
        let x = uv.x.rem_euclid(1.0) * self.panorama.width as f32 - 0.5;
        let y = uv.y.clamp(0.0, 1.0) * self.panorama.height as f32 - 0.5;
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let tx = x - x.floor();
        let ty = y - y.floor();
        let sample = |source_x: i32, source_y: i32| {
            let wrapped_x = source_x.rem_euclid(width) as u32;
            let clamped_y = source_y.clamp(0, height - 1) as u32;
            self.panorama
                .pixels
                .color((clamped_y * self.panorama.width + wrapped_x) as usize)
        };
        let top = mix_color(sample(x0, y0), sample(x0 + 1, y0), tx);
        let bottom = mix_color(sample(x0, y0 + 1), sample(x0 + 1, y0 + 1), tx);
        mix_color(top, bottom, ty)
    }

    fn decode_radiance(&self, sample: Color) -> Vec3 {
        match self.encoding {
            EnvironmentEncoding::LinearHdr => sample.to_vec3(),
            EnvironmentEncoding::Srgb => Vec3::new(
                srgb_to_linear(sample.r),
                srgb_to_linear(sample.g),
                srgb_to_linear(sample.b),
            ),
        }
    }

    fn build_irradiance_sh(&self) -> [[f32; 3]; 9] {
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 32;
        const BAND_CONVOLUTION: [f32; 3] = [PI, 2.0 * PI / 3.0, PI / 4.0];
        let mut coefficients = [[0.0_f32; 3]; 9];
        let mut weight_sum = 0.0;
        for y in 0..HEIGHT {
            let theta = PI * (y as f32 + 0.5) / HEIGHT as f32;
            let sin_theta = theta.sin();
            let direction_y = theta.cos();
            for x in 0..WIDTH {
                let phi = TAU * (x as f32 + 0.5) / WIDTH as f32 - PI;
                let direction =
                    Vec3::new(sin_theta * phi.cos(), direction_y, sin_theta * phi.sin());
                let radiance = self.sample_radiance(direction);
                let basis = sh_basis(direction);
                for index in 0..9 {
                    coefficients[index][0] += radiance.x * basis[index] * sin_theta;
                    coefficients[index][1] += radiance.y * basis[index] * sin_theta;
                    coefficients[index][2] += radiance.z * basis[index] * sin_theta;
                }
                weight_sum += sin_theta;
            }
        }
        let normalization = 4.0 * PI / weight_sum.max(f32::EPSILON);
        for (index, coefficient) in coefficients.iter_mut().enumerate() {
            let band = if index == 0 {
                0
            } else if index <= 3 {
                1
            } else {
                2
            };
            for channel in coefficient {
                *channel *= normalization * BAND_CONVOLUTION[band];
            }
        }
        coefficients
    }
}

fn mix_color(a: Color, b: Color, t: f32) -> Color {
    Color::new(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

fn sh_basis(direction: Vec3) -> [f32; 9] {
    let x = direction.x;
    let y = direction.y;
    let z = direction.z;
    [
        0.282_095,
        0.488_603 * z,
        0.488_603 * y,
        0.488_603 * x,
        1.092_548 * x * z,
        1.092_548 * z * y,
        0.315_392 * (3.0 * y * y - 1.0),
        1.092_548 * x * y,
        0.546_274 * (x * x - z * z),
    ]
}

fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

impl Material {
    pub fn to_asset_desc(&self) -> MaterialDesc {
        let mut textures = vec![
            TextureDesc {
                name: format!("{}_normal_bent_bc7", self.name),
                format: TextureFormat::Bc7,
                role: TextureRole::NormalsAndBentNormals,
            },
            TextureDesc {
                name: format!("{}_base_ao_bc3", self.name),
                format: TextureFormat::Bc3,
                role: TextureRole::BaseColorAndAo,
            },
            TextureDesc {
                name: format!("{}_spec_rough_bc3", self.name),
                format: TextureFormat::Bc3,
                role: TextureRole::SpecularRoughnessHeight,
            },
        ];
        if self.flags.alpha_tested {
            textures.push(TextureDesc {
                name: format!("{}_opacity_bc4", self.name),
                format: TextureFormat::Bc4,
                role: TextureRole::Opacity,
            });
        }

        MaterialDesc {
            name: self.name.clone(),
            textures,
            texture_sample_count: self.texture_sample_count,
            runtime_texture_blending: false,
            requires_specular_occlusion: self.specular_intensity > 0.15,
            has_bent_normals: self.flags.uses_bent_normals,
            alpha_tested: self.flags.alpha_tested,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub previous_position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec3,
    pub tangent_sign: f32,
    pub uv: Vec2,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<[u32; 3]>,
    pub material_index: usize,
    pub transform: Mat4,
    pub shadow_class: ShadowCasterClass,
    pub is_floor_or_ground: bool,
    pub prepass_eligible: bool,
    pub reflection_proxy: bool,
    pub cloth: Option<ClothDefinition>,
}

impl Mesh {
    pub fn to_asset_desc(&self) -> MeshDesc {
        MeshDesc {
            name: self.name.clone(),
            material_index: self.material_index,
            is_floor_or_ground: self.is_floor_or_ground,
            shadow_class: self.shadow_class,
            participates_in_prepass: self.prepass_eligible,
            writes_motion_vectors_in_prepass: false,
            long_thin_triangles: false,
            dense_alpha_cards: false,
        }
    }

    pub fn bake_transform(&mut self) {
        if self.transform == Mat4::IDENTITY {
            return;
        }
        for vertex in &mut self.vertices {
            let position = self.transform.mul_vec4(Vec4::new(
                vertex.position.x,
                vertex.position.y,
                vertex.position.z,
                1.0,
            ));
            let previous = self.transform.mul_vec4(Vec4::new(
                vertex.previous_position.x,
                vertex.previous_position.y,
                vertex.previous_position.z,
                1.0,
            ));
            let normal = self.transform.mul_vec4(Vec4::new(
                vertex.normal.x,
                vertex.normal.y,
                vertex.normal.z,
                0.0,
            ));
            let tangent = self.transform.mul_vec4(Vec4::new(
                vertex.tangent.x,
                vertex.tangent.y,
                vertex.tangent.z,
                0.0,
            ));
            vertex.position = position.xyz();
            vertex.previous_position = previous.xyz();
            vertex.normal = normal.xyz().normalize_or_zero();
            vertex.tangent = tangent.xyz().normalize_or_zero();
        }
        self.transform = Mat4::IDENTITY;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Directional {
        direction: Vec3,
    },
    Local {
        position: Vec3,
        radius: f32,
    },
    Spot {
        position: Vec3,
        direction: Vec3,
        radius: f32,
        inner_angle_radians: f32,
        outer_angle_radians: f32,
    },
}

impl LightKind {
    /// The same light placed at `position`; directional lights have no position.
    pub fn at_position(self, position: Vec3) -> Self {
        match self {
            Self::Directional { direction } => Self::Directional { direction },
            Self::Local { radius, .. } => Self::Local { position, radius },
            Self::Spot {
                direction,
                radius,
                inner_angle_radians,
                outer_angle_radians,
                ..
            } => Self::Spot {
                position,
                direction,
                radius,
                inner_angle_radians,
                outer_angle_radians,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub name: String,
    pub kind: LightKind,
    pub color: Color,
    pub intensity: f32,
    pub casts_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub vertical_fov_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn view_projection(self, aspect: f32) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
            * Mat4::perspective_reverse_rh(self.vertical_fov_radians, aspect, self.near, self.far)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderScene {
    pub camera: Camera,
    pub materials: Vec<Material>,
    pub meshes: Vec<Mesh>,
    pub lights: Vec<Light>,
    pub ambient_color: Color,
    pub environment_map: EnvironmentMap,
    pub environment: EnvironmentProfile,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureProfile {
    pub min: f32,
    pub max: f32,
    pub compensation: f32,
    pub white_balance_strength: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvironmentProfile {
    pub precipitation: f32,
    pub surface_wetness: f32,
    pub wind: f32,
    pub exposure: ExposureProfile,
}

impl EnvironmentProfile {
    pub const fn rainy() -> Self {
        Self {
            precipitation: 1.0,
            surface_wetness: 1.0,
            wind: 0.18,
            exposure: ExposureProfile {
                min: 0.35,
                max: 3.5,
                compensation: 0.0,
                white_balance_strength: 0.65,
            },
        }
    }

    pub const fn dry_daylight() -> Self {
        Self {
            precipitation: 0.0,
            surface_wetness: 0.0,
            wind: 0.08,
            exposure: ExposureProfile {
                min: 0.35,
                max: 3.0,
                compensation: 0.0,
                white_balance_strength: 0.20,
            },
        }
    }
}

impl RenderScene {
    pub fn asset_descriptors(&self) -> (Vec<MaterialDesc>, Vec<MeshDesc>) {
        (
            self.materials.iter().map(Material::to_asset_desc).collect(),
            self.meshes.iter().map(Mesh::to_asset_desc).collect(),
        )
    }

    pub fn environment_radiance(&self, direction: Vec3) -> Vec3 {
        self.environment_map.sample_radiance(direction)
    }

    pub fn environment_irradiance(&self, normal: Vec3) -> Vec3 {
        self.environment_map.diffuse_irradiance(normal) + self.ambient_color.to_vec3()
    }
}
