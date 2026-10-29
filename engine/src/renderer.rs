use crate::asset::{validate_asset, AssetIssue, ShadowCasterClass};
use crate::graph::{build_baseline_frame_graph, GraphIssue};
use crate::math::{lerp3, saturate, Color, Vec2, Vec3, Vec4};
use crate::perf::{
    blue_noise_hash, edge_stencil_4x4, gt_tonemap_rgba8, kawase_blur_rgba32f, reduce_hiz_rgba16,
};
use crate::quality::QualityTier;
use crate::scene::{LightKind, MaterialClass, RenderScene};
use std::time::Instant;

const INV_PI: f32 = std::f32::consts::FRAC_1_PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugView {
    Beauty,
    Depth,
    Normals,
    Roughness,
    MaterialId,
    ShadowMask,
    AmbientOcclusion,
    MotionVectors,
    Reflections,
    EdgeStencil,
    Overdraw,
    TextureCount,
    BentNormalOcclusion,
    QuadUtilization,
    ShadowCasters,
    LightVolumes,
    HistoryValidity,
}

impl DebugView {
    pub const ALL: [Self; 17] = [
        Self::Beauty,
        Self::Depth,
        Self::Normals,
        Self::Roughness,
        Self::MaterialId,
        Self::ShadowMask,
        Self::AmbientOcclusion,
        Self::MotionVectors,
        Self::Reflections,
        Self::EdgeStencil,
        Self::Overdraw,
        Self::TextureCount,
        Self::BentNormalOcclusion,
        Self::QuadUtilization,
        Self::ShadowCasters,
        Self::LightVolumes,
        Self::HistoryValidity,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Self::Beauty => "beauty",
            Self::Depth => "depth",
            Self::Normals => "normals",
            Self::Roughness => "roughness",
            Self::MaterialId => "material-id",
            Self::ShadowMask => "shadow-mask",
            Self::AmbientOcclusion => "ambient-occlusion",
            Self::MotionVectors => "motion-vectors",
            Self::Reflections => "reflections",
            Self::EdgeStencil => "edge-stencil",
            Self::Overdraw => "overdraw",
            Self::TextureCount => "texture-count",
            Self::BentNormalOcclusion => "bent-normal-occlusion",
            Self::QuadUtilization => "quad-utilization",
            Self::ShadowCasters => "shadow-casters",
            Self::LightVolumes => "light-volumes",
            Self::HistoryValidity => "history-validity",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPath {
    Deferred,
    ForwardPlus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
    pub quality: QualityTier,
    pub path: RenderPath,
    pub debug_view: DebugView,
    pub exposure: f32,
    pub frame_index: u32,
    pub enable_software_rt: bool,
    pub enable_ssr: bool,
    pub enable_fog: bool,
    pub enable_bloom: bool,
    pub enable_persistence: bool,
}

impl RenderConfig {
    pub fn headless_1080p() -> Self {
        Self {
            width: 1920,
            height: 1080,
            quality: QualityTier::High,
            path: RenderPath::Deferred,
            debug_view: DebugView::Beauty,
            exposure: 1.0,
            frame_index: 0,
            enable_software_rt: true,
            enable_ssr: true,
            enable_fog: true,
            enable_bloom: true,
            enable_persistence: true,
        }
    }

    pub fn tiny_test() -> Self {
        Self {
            width: 96,
            height: 64,
            exposure: 1.0,
            ..Self::headless_1080p()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PassReport {
    pub name: &'static str,
    pub budget_ms: f32,
    pub cpu_ms: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderWarning {
    Asset(AssetIssue),
    Graph(GraphIssue),
    SsrCostExceededSoftwareRt {
        ssr_ms: f32,
        software_rt_ms: f32,
    },
    DrawTextureLimit {
        mesh: String,
        texture_count: usize,
        limit: usize,
    },
    ShadowFloorExcluded {
        mesh: String,
    },
    OverwrittenExpensiveMaterialArea {
        ratio: f32,
    },
    EmptySmaaEdges,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderError {
    InvalidDimensions,
    MissingMaterial { mesh: String, material_index: usize },
    KernelFailure,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOutput {
    pub width: u32,
    pub height: u32,
    pub color: Vec<[u8; 4]>,
    pub hdr: Vec<[f32; 4]>,
    pub pass_reports: Vec<PassReport>,
    pub warnings: Vec<RenderWarning>,
    pub graph_passes: usize,
    pub graph_resources: usize,
}

impl RenderOutput {
    pub fn ppm_bytes(&self) -> Vec<u8> {
        let mut bytes = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        bytes.reserve(self.color.len() * 3);
        for px in &self.color {
            bytes.extend_from_slice(&px[..3]);
        }
        bytes
    }

    pub fn average_luminance(&self) -> f32 {
        if self.hdr.is_empty() {
            return 0.0;
        }
        let sum: f32 = self
            .hdr
            .iter()
            .map(|p| p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722)
            .sum();
        sum / self.hdr.len() as f32
    }
}

#[derive(Clone, Copy, Debug)]
struct GPixel {
    covered: bool,
    material_index: usize,
    material_id: u8,
    albedo: Color,
    opacity: f32,
    normal: Vec3,
    world: Vec3,
    roughness: f32,
    specular: f32,
    cavity: f32,
    velocity: Vec2,
    texture_count: usize,
    glass: bool,
    emissive: bool,
    responsive_aa: bool,
    shadow_class: ShadowCasterClass,
}

impl Default for GPixel {
    fn default() -> Self {
        Self {
            covered: false,
            material_index: 0,
            material_id: 0,
            albedo: Color::BLACK,
            opacity: 1.0,
            normal: Vec3::Y,
            world: Vec3::ZERO,
            roughness: 1.0,
            specular: 0.0,
            cavity: 1.0,
            velocity: Vec2::ZERO,
            texture_count: 0,
            glass: false,
            emissive: false,
            responsive_aa: false,
            shadow_class: ShadowCasterClass::None,
        }
    }
}

#[derive(Debug)]
struct FrameBuffers {
    depth: Vec<f32>,
    gbuffer: Vec<GPixel>,
    overdraw: Vec<u16>,
    shadow_mask: Vec<f32>,
    ao: Vec<f32>,
    reflections: Vec<Color>,
    lighting: Vec<[f32; 4]>,
    edges: Vec<u8>,
}

impl FrameBuffers {
    fn new(pixels: usize) -> Self {
        Self {
            depth: vec![0.0; pixels],
            gbuffer: vec![GPixel::default(); pixels],
            overdraw: vec![0; pixels],
            shadow_mask: vec![1.0; pixels],
            ao: vec![1.0; pixels],
            reflections: vec![Color::BLACK; pixels],
            lighting: vec![[0.0; 4]; pixels],
            edges: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScreenVertex {
    x: f32,
    y: f32,
    depth: f32,
    world: Vec3,
    previous_screen: Vec2,
    normal: Vec3,
    tangent: Vec3,
    tangent_sign: f32,
    color: Color,
    uv: Vec2,
}

pub struct CpuRenderer {
    pub config: RenderConfig,
    history: Vec<[f32; 4]>,
}

impl CpuRenderer {
    pub fn new(config: RenderConfig) -> Self {
        Self {
            config,
            history: Vec::new(),
        }
    }

    pub fn render(&mut self, scene: &RenderScene) -> Result<RenderOutput, RenderError> {
        if self.config.width == 0 || self.config.height == 0 {
            return Err(RenderError::InvalidDimensions);
        }
        let pixels = (self.config.width as usize)
            .checked_mul(self.config.height as usize)
            .ok_or(RenderError::InvalidDimensions)?;
        let mut buffers = FrameBuffers::new(pixels);
        let mut reports = Vec::new();
        let mut warnings = self.validate_inputs(scene);
        let graph = build_baseline_frame_graph();
        warnings.extend(graph.validate().into_iter().map(RenderWarning::Graph));

        Self::run_pass(&mut reports, "frame-start-depth-motion-clear", 0.1, || {
            buffers.depth.fill(0.0);
            for pixel in &mut buffers.gbuffer {
                pixel.covered = false;
                pixel.velocity = Vec2::ZERO;
            }
            Ok(())
        })?;

        Self::run_pass(&mut reports, "partial-depth-prepass", 0.5, || {
            self.raster_scene(scene, &mut buffers, true)?;
            Ok(())
        })?;

        Self::run_pass(&mut reports, "base-pass-deferred-materials", 3.0, || {
            self.raster_scene(scene, &mut buffers, false)
        })?;

        Self::run_pass(&mut reports, "depth-copy-and-hiz", 0.3, || {
            let _ = reduce_hiz_rgba16(&buffers.depth, self.config.width, self.config.height)
                .map_err(|_| RenderError::KernelFailure)?;
            Ok(())
        })?;

        Self::run_pass(
            &mut reports,
            "stencil-restricted-shadow-projection",
            1.0,
            || {
                self.compute_shadow_mask(scene, &mut buffers, &mut warnings);
                Ok(())
            },
        )?;

        Self::run_pass(&mut reports, "multi-scale-ao-contact-shadows", 0.8, || {
            self.compute_ao_contact(scene, &mut buffers);
            Ok(())
        })?;

        let start = Instant::now();
        self.compute_reflections(scene, &mut buffers, true);
        let ssr_ms = start.elapsed().as_secs_f32() * 1000.0;
        reports.push(PassReport {
            name: "screen-space-reflections",
            budget_ms: 0.7,
            cpu_ms: ssr_ms,
        });
        let start = Instant::now();
        self.compute_reflections(scene, &mut buffers, false);
        let software_rt_ms = start.elapsed().as_secs_f32() * 1000.0;
        reports.push(PassReport {
            name: "software-rt-reflection-supplement",
            budget_ms: 1.3,
            cpu_ms: software_rt_ms,
        });
        if ssr_ms > software_rt_ms * 1.2 && software_rt_ms > 0.0 {
            warnings.push(RenderWarning::SsrCostExceededSoftwareRt {
                ssr_ms,
                software_rt_ms,
            });
        }

        Self::run_pass(&mut reports, "albedo-free-deferred-lighting", 1.5, || {
            self.compute_lighting(scene, &mut buffers);
            Ok(())
        })?;

        Self::run_pass(&mut reports, "blue-noise-volumetric-fog", 1.0, || {
            if self.config.enable_fog {
                self.apply_fog(scene, &mut buffers);
            }
            Ok(())
        })?;

        Self::run_pass(&mut reports, "smaa-compute-edge-detect", 0.5, || {
            buffers.edges =
                edge_stencil_4x4(&buffers.depth, self.config.width, self.config.height, 0.003)
                    .map_err(|_| RenderError::KernelFailure)?;
            if !buffers.edges.iter().any(|&e| e != 0) {
                warnings.push(RenderWarning::EmptySmaaEdges);
            }
            Ok(())
        })?;

        // T2x history must contain this frame's unblended spatial result. Saving
        // a temporally blended frame here would create an accumulation buffer.
        let current_history = buffers.lighting.clone();

        Self::run_pass(&mut reports, "kawase-bloom-and-gt-tonemap", 1.0, || {
            if self.config.enable_bloom {
                self.apply_bloom(&mut buffers)?;
            }
            if self.config.enable_persistence {
                self.apply_persistence(&mut buffers.lighting, &buffers.edges, &buffers.gbuffer);
            }
            Ok(())
        })?;

        let mut color = gt_tonemap_rgba8(&buffers.lighting, self.config.exposure)
            .map_err(|_| RenderError::KernelFailure)?;

        Self::run_pass(&mut reports, "smaa-persistence-resolve", 0.5, || {
            self.apply_smaa(&buffers, &mut color);
            Ok(())
        })?;

        color = self.debug_or_beauty(scene, &buffers, color)?;
        self.history = current_history;

        Ok(RenderOutput {
            width: self.config.width,
            height: self.config.height,
            color,
            hdr: buffers.lighting,
            pass_reports: reports,
            warnings,
            graph_passes: graph.pass_count(),
            graph_resources: graph.resource_count(),
        })
    }

    fn run_pass<T>(
        reports: &mut Vec<PassReport>,
        name: &'static str,
        budget_ms: f32,
        pass: impl FnOnce() -> Result<T, RenderError>,
    ) -> Result<T, RenderError> {
        let start = Instant::now();
        let result = pass();
        reports.push(PassReport {
            name,
            budget_ms,
            cpu_ms: start.elapsed().as_secs_f32() * 1000.0,
        });
        result
    }

    fn validate_inputs(&self, scene: &RenderScene) -> Vec<RenderWarning> {
        let (materials, meshes) = scene.asset_descriptors();
        let mut warnings: Vec<_> = validate_asset(&materials, &meshes, self.config.quality)
            .into_iter()
            .map(RenderWarning::Asset)
            .collect();
        for mesh in &scene.meshes {
            if mesh.is_floor_or_ground && mesh.shadow_class == ShadowCasterClass::None {
                warnings.push(RenderWarning::ShadowFloorExcluded {
                    mesh: mesh.name.clone(),
                });
            }
            if let Some(material) = scene.materials.get(mesh.material_index) {
                let limit = self.config.quality.material_texture_limit();
                if material.texture_sample_count > limit {
                    warnings.push(RenderWarning::DrawTextureLimit {
                        mesh: mesh.name.clone(),
                        texture_count: material.texture_sample_count,
                        limit,
                    });
                }
            }
        }
        warnings
    }

    fn raster_scene(
        &self,
        scene: &RenderScene,
        buffers: &mut FrameBuffers,
        depth_only: bool,
    ) -> Result<(), RenderError> {
        let mut draw_order: Vec<usize> = (0..scene.meshes.len()).collect();
        draw_order.sort_by_key(|&i| scene.meshes[i].prepass_eligible);
        if depth_only {
            draw_order.retain(|&i| {
                let mesh = &scene.meshes[i];
                let Some(material) = scene.materials.get(mesh.material_index) else {
                    return false;
                };
                mesh.prepass_eligible && !material.flags.alpha_tested
            });
        }

        for mesh_index in draw_order {
            let mesh = &scene.meshes[mesh_index];
            let material = scene.materials.get(mesh.material_index).ok_or_else(|| {
                RenderError::MissingMaterial {
                    mesh: mesh.name.clone(),
                    material_index: mesh.material_index,
                }
            })?;
            if !depth_only && material.flags.alpha_tested && mesh.prepass_eligible {
                continue;
            }
            for tri in &mesh.indices {
                let v0 = mesh.vertices[tri[0] as usize];
                let v1 = mesh.vertices[tri[1] as usize];
                let v2 = mesh.vertices[tri[2] as usize];
                if let Some(screen) = self.project_triangle(scene, mesh.transform, [v0, v1, v2]) {
                    self.raster_triangle(
                        &screen,
                        mesh.material_index,
                        mesh.shadow_class,
                        scene,
                        buffers,
                        depth_only,
                    );
                }
            }
        }
        Ok(())
    }

    fn project_triangle(
        &self,
        scene: &RenderScene,
        model: crate::math::Mat4,
        vertices: [crate::scene::Vertex; 3],
    ) -> Option<[ScreenVertex; 3]> {
        let aspect = self.config.width as f32 / self.config.height as f32;
        let vp = scene.camera.view_projection(aspect);
        let mut out = [ScreenVertex {
            x: 0.0,
            y: 0.0,
            depth: 0.0,
            world: Vec3::ZERO,
            previous_screen: Vec2::ZERO,
            normal: Vec3::Y,
            tangent: Vec3::X,
            tangent_sign: 1.0,
            color: Color::WHITE,
            uv: Vec2::ZERO,
        }; 3];

        for (i, vertex) in vertices.iter().enumerate() {
            let world4 = model.mul_vec4(Vec4::new(
                vertex.position.x,
                vertex.position.y,
                vertex.position.z,
                1.0,
            ));
            let clip = vp.mul_vec4(world4);
            if clip.w <= 1.0e-5 {
                return None;
            }
            let inv_w = 1.0 / clip.w;
            let ndc = Vec3::new(clip.x * inv_w, clip.y * inv_w, clip.z * inv_w);
            let previous_world = model.mul_vec4(Vec4::new(
                vertex.previous_position.x,
                vertex.previous_position.y,
                vertex.previous_position.z,
                1.0,
            ));
            let previous_clip = vp.mul_vec4(previous_world);
            let previous_inv_w = 1.0 / previous_clip.w.max(1.0e-5);
            let previous_ndc = Vec2::new(
                previous_clip.x * previous_inv_w,
                previous_clip.y * previous_inv_w,
            );
            let n4 = model.mul_vec4(Vec4::new(
                vertex.normal.x,
                vertex.normal.y,
                vertex.normal.z,
                0.0,
            ));
            let t4 = model.mul_vec4(Vec4::new(
                vertex.tangent.x,
                vertex.tangent.y,
                vertex.tangent.z,
                0.0,
            ));
            out[i] = ScreenVertex {
                x: (ndc.x * 0.5 + 0.5) * (self.config.width - 1) as f32,
                y: (1.0 - (ndc.y * 0.5 + 0.5)) * (self.config.height - 1) as f32,
                depth: saturate(ndc.z),
                world: world4.xyz(),
                previous_screen: Vec2::new(
                    (previous_ndc.x * 0.5 + 0.5) * (self.config.width - 1) as f32,
                    (1.0 - (previous_ndc.y * 0.5 + 0.5)) * (self.config.height - 1) as f32,
                ),
                normal: n4.xyz().normalize_or_zero(),
                tangent: t4.xyz().normalize_or_zero(),
                tangent_sign: vertex.tangent_sign,
                color: vertex.color,
                uv: vertex.uv,
            };
        }
        Some(out)
    }

    fn raster_triangle(
        &self,
        tri: &[ScreenVertex; 3],
        material_index: usize,
        shadow_class: ShadowCasterClass,
        scene: &RenderScene,
        buffers: &mut FrameBuffers,
        depth_only: bool,
    ) {
        let min_x = tri
            .iter()
            .map(|v| v.x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_x = tri
            .iter()
            .map(|v| v.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min((self.config.width - 1) as f32) as u32;
        let min_y = tri
            .iter()
            .map(|v| v.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as u32;
        let max_y = tri
            .iter()
            .map(|v| v.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .min((self.config.height - 1) as f32) as u32;
        let area = edge(tri[0].x, tri[0].y, tri[1].x, tri[1].y, tri[2].x, tri[2].y);
        if area.abs() < 1.0e-5 {
            return;
        }
        let material = &scene.materials[material_index];

        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let w0 = edge(tri[1].x, tri[1].y, tri[2].x, tri[2].y, px, py) / area;
                let w1 = edge(tri[2].x, tri[2].y, tri[0].x, tri[0].y, px, py) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -0.0001 || w1 < -0.0001 || w2 < -0.0001 {
                    continue;
                }
                let idx = self.index(x, y);
                let depth = w0 * tri[0].depth + w1 * tri[1].depth + w2 * tri[2].depth;
                if depth + 0.0001 < buffers.depth[idx] {
                    continue;
                }
                if !depth_only {
                    buffers.overdraw[idx] = buffers.overdraw[idx].saturating_add(1);
                }
                buffers.depth[idx] = depth;
                if depth_only {
                    continue;
                }
                let geometric_normal =
                    (tri[0].normal * w0 + tri[1].normal * w1 + tri[2].normal * w2)
                        .normalize_or_zero();
                let tangent = (tri[0].tangent * w0 + tri[1].tangent * w1 + tri[2].tangent * w2)
                    .normalize_or_zero();
                let tangent = (tangent - geometric_normal * tangent.dot(geometric_normal))
                    .normalize_or_zero();
                let tangent = if tangent.length() > 0.001 {
                    tangent
                } else {
                    Vec3::X
                };
                let tangent_sign =
                    tri[0].tangent_sign * w0 + tri[1].tangent_sign * w1 + tri[2].tangent_sign * w2;
                let world = tri[0].world * w0 + tri[1].world * w1 + tri[2].world * w2;
                let previous_screen = tri[0].previous_screen * w0
                    + tri[1].previous_screen * w1
                    + tri[2].previous_screen * w2;
                let velocity = Vec2::new(
                    (px - previous_screen.x) / self.config.width as f32,
                    (py - previous_screen.y) / self.config.height as f32,
                );
                let vertex_color = tri[0].color * w0 + tri[1].color * w1 + tri[2].color * w2;
                let uv = tri[0].uv * w0 + tri[1].uv * w1 + tri[2].uv * w2;
                let texture_color = material
                    .base_color_texture
                    .as_ref()
                    .map(|texture| texture.sample_repeat(uv))
                    .unwrap_or(Color::WHITE);
                let source_albedo = material.base_color * vertex_color * texture_color;
                let opacity = if material.flags.glass {
                    source_albedo.a.clamp(0.04, 1.0)
                } else {
                    1.0
                };
                let under_albedo = if material.flags.glass {
                    if buffers.gbuffer[idx].covered {
                        buffers.gbuffer[idx].albedo.to_vec3()
                    } else {
                        transparent_underlay(world)
                    }
                } else {
                    source_albedo.to_vec3()
                };
                let blended_albedo = if material.flags.glass {
                    Color::from_vec3(
                        lerp3(under_albedo, source_albedo.to_vec3(), opacity),
                        opacity,
                    )
                } else {
                    source_albedo
                };
                let sampled_normal = material
                    .normal_texture
                    .as_ref()
                    .map(|texture| texture.sample_repeat(uv))
                    .unwrap_or(Color::new(0.5, 0.5, 1.0, 1.0));
                let pbr = material
                    .pbr_texture
                    .as_ref()
                    .map(|texture| texture.sample_repeat(uv))
                    .unwrap_or(Color::new(1.0, 1.0, 1.0, 1.0));
                let local_normal = Vec3::new(
                    sampled_normal.r * 2.0 - 1.0,
                    sampled_normal.g * 2.0 - 1.0,
                    sampled_normal.b * 2.0 - 1.0,
                );
                let tangent_w = if tangent_sign < 0.0 { -1.0 } else { 1.0 };
                let bitangent = geometric_normal.cross(tangent).normalize_or_zero() * tangent_w;
                let normal = (tangent * local_normal.x
                    + bitangent * local_normal.y
                    + geometric_normal * local_normal.z)
                    .normalize_or_zero();
                buffers.gbuffer[idx] = GPixel {
                    covered: true,
                    material_index,
                    material_id: material.material_id,
                    albedo: blended_albedo,
                    opacity,
                    normal,
                    world,
                    roughness: material.roughness * pbr.g,
                    specular: material.specular_intensity,
                    cavity: material.cavity * pbr.r,
                    velocity,
                    texture_count: material.texture_sample_count,
                    glass: material.flags.glass,
                    emissive: material.flags.emissive,
                    responsive_aa: material.flags.responsive_aa,
                    shadow_class,
                };
            }
        }
    }

    fn compute_shadow_mask(
        &self,
        scene: &RenderScene,
        buffers: &mut FrameBuffers,
        warnings: &mut Vec<RenderWarning>,
    ) {
        for (idx, pixel) in buffers.gbuffer.iter().enumerate() {
            if !pixel.covered {
                continue;
            }
            let mut shadow = 1.0;
            if pixel.world.y < -0.72 {
                for mesh in &scene.meshes {
                    if mesh.is_floor_or_ground || mesh.shadow_class == ShadowCasterClass::None {
                        continue;
                    }
                    let center = mesh.transform.mul_vec4(Vec4::new(0.0, 0.0, 0.0, 1.0)).xyz();
                    let dx = (pixel.world.x - center.x).abs();
                    let dz = (pixel.world.z - center.z).abs();
                    if dx < 0.65 && dz < 0.35 {
                        shadow *= 0.58;
                    }
                }
            }
            buffers.shadow_mask[idx] = shadow;
        }

        let expensive = buffers
            .gbuffer
            .iter()
            .zip(&buffers.overdraw)
            .filter(|(g, overdraw)| g.covered && g.texture_count >= 4 && **overdraw > 1)
            .count();
        let covered = buffers
            .gbuffer
            .iter()
            .filter(|g| g.covered && g.texture_count >= 4)
            .count()
            .max(1);
        let ratio = expensive as f32 / covered as f32;
        if ratio > 0.15 {
            warnings.push(RenderWarning::OverwrittenExpensiveMaterialArea { ratio });
        }
    }

    fn compute_ao_contact(&self, scene: &RenderScene, buffers: &mut FrameBuffers) {
        let width = self.config.width as i32;
        let height = self.config.height as i32;
        for y in 0..height {
            for x in 0..width {
                let idx = self.index(x as u32, y as u32);
                if !buffers.gbuffer[idx].covered {
                    continue;
                }
                let z = buffers.depth[idx];
                let mut hits = 0;
                let mut total = 0;
                for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-2, 1), (2, -1)] {
                    let sx = x + ox;
                    let sy = y + oy;
                    if sx < 0 || sy < 0 || sx >= width || sy >= height {
                        continue;
                    }
                    total += 1;
                    let nz = buffers.depth[self.index(sx as u32, sy as u32)];
                    if nz < z - 0.015 {
                        hits += 1;
                    }
                }
                let mut ao = 1.0 - hits as f32 / total.max(1) as f32 * 0.45;
                let material = &scene.materials[buffers.gbuffer[idx].material_index];
                if material.flags.receives_contact_shadows && buffers.gbuffer[idx].world.y < -0.65 {
                    ao *= 0.9;
                }
                buffers.ao[idx] = ao * buffers.gbuffer[idx].cavity;
            }
        }
    }

    fn compute_reflections(&self, scene: &RenderScene, buffers: &mut FrameBuffers, ssr: bool) {
        if ssr && !self.config.enable_ssr {
            return;
        }
        if !ssr && !self.config.enable_software_rt {
            return;
        }
        let width = self.config.width;
        if ssr {
            for y in (0..self.config.height).step_by(2) {
                for x in (0..width).step_by(2) {
                    let mirror_x = width - 1 - x;
                    let sample = self.index(mirror_x, y);
                    if !buffers.gbuffer[sample].covered {
                        continue;
                    }
                    let source = buffers.gbuffer[sample].albedo;
                    for oy in 0..2 {
                        for ox in 0..2 {
                            let target_x = x + ox;
                            let target_y = y + oy;
                            if target_x >= width || target_y >= self.config.height {
                                continue;
                            }
                            let target = self.index(target_x, target_y);
                            let g = buffers.gbuffer[target];
                            if g.covered && (g.roughness <= 0.55 || g.glass) {
                                buffers.reflections[target] = source * (1.0 - g.roughness) * 0.45;
                            }
                        }
                    }
                }
            }
            return;
        }

        for y in 0..self.config.height {
            for x in 0..width {
                let idx = self.index(x, y);
                let g = buffers.gbuffer[idx];
                if !g.covered || (g.roughness > 0.55 && !g.glass) {
                    continue;
                }
                let noise = (blue_noise_hash(x, y, self.config.frame_index, 0xA53C_91E9) & 255)
                    as f32
                    / 255.0;
                let view_ray = (g.world - scene.camera.position).normalize_or_zero();
                let reflected = reflect(view_ray, g.normal).normalize_or_zero();
                let sky = Color::from_vec3(scene.environment_radiance(reflected), 1.0);
                let emissive = scene
                    .materials
                    .iter()
                    .filter(|m| m.flags.emissive)
                    .fold(Color::BLACK, |acc, m| acc + m.emission * 0.08);
                let proxy = trace_alley_proxy(scene, g.world + g.normal * 0.045, reflected);
                let local = local_reflection_radiance(scene, g.world, reflected, g.glass);
                let ripple = 0.86 + noise * 0.18;
                let glass_boost = if g.glass { 1.62 } else { 1.0 };
                let glass_line = if g.glass && (x + y * 3) % 7 == 0 {
                    1.22
                } else {
                    1.0
                };
                let candidate = (sky + emissive * 0.08 + proxy * 1.16 + local * 1.15)
                    * (1.0 - g.roughness)
                    * glass_boost
                    * glass_line
                    * ripple;
                let existing = buffers.reflections[idx];
                buffers.reflections[idx] = if g.glass {
                    existing * 0.20 + candidate
                } else if existing.luminance() < 0.001 {
                    candidate
                } else {
                    existing * 0.58 + candidate * 0.42
                };
            }
        }
    }

    fn compute_lighting(&self, scene: &RenderScene, buffers: &mut FrameBuffers) {
        for idx in 0..buffers.gbuffer.len() {
            let g = buffers.gbuffer[idx];
            if !g.covered {
                let x = idx as u32 % self.config.width;
                let y = idx as u32 / self.config.width;
                let sky = scene.environment_radiance(camera_ray(
                    scene,
                    x,
                    y,
                    self.config.width,
                    self.config.height,
                ));
                buffers.lighting[idx] = [sky.x, sky.y, sky.z, 0.18_f32.min(sky_luminance(sky))];
                continue;
            }
            let material = &scene.materials[g.material_index];
            let view = (scene.camera.position - g.world).normalize_or_zero();
            let mut diffuse_irradiance = indirect_diffuse_irradiance(scene, g.normal) * INV_PI;
            let mut specular = Vec3::ZERO;
            let ao = buffers.ao[idx];

            for light in &scene.lights {
                let (l, attenuation) = match light.kind {
                    LightKind::Directional { direction } => ((-direction).normalize_or_zero(), 1.0),
                    LightKind::Local { position, radius } => {
                        let to_light = position - g.world;
                        let dist = to_light.length();
                        if dist >= radius {
                            continue;
                        }
                        let a = 1.0 - dist / radius;
                        (to_light / dist.max(1.0e-4), a * a)
                    }
                    LightKind::Spot {
                        position,
                        direction,
                        radius,
                        inner_angle_radians,
                        outer_angle_radians,
                    } => {
                        let to_light = position - g.world;
                        let dist = to_light.length();
                        if dist >= radius {
                            continue;
                        }
                        let surface_to_light = to_light / dist.max(1.0e-4);
                        let light_to_surface = -surface_to_light;
                        let inner = inner_angle_radians.cos();
                        let outer = outer_angle_radians.cos();
                        let cone = saturate(
                            (light_to_surface.dot(direction.normalize_or_zero()) - outer)
                                / (inner - outer).max(1.0e-4),
                        );
                        let smooth_cone = cone * cone * (3.0 - 2.0 * cone);
                        let practical_fill = 0.42 + smooth_cone * 0.58;
                        let a = 1.0 - dist / radius;
                        (surface_to_light, a * a * practical_fill)
                    }
                };
                let ndotl = g.normal.dot(l).max(0.0);
                if ndotl <= 0.0 {
                    continue;
                }
                let shadow = if light.casts_shadow {
                    buffers.shadow_mask[idx]
                } else {
                    1.0
                };
                let light_rgb = light.color.to_vec3() * (light.intensity * attenuation * shadow);
                let burley = burley_diffuse(g.normal, view, l, g.roughness);
                diffuse_irradiance += light_rgb * burley;

                let h = (l + view).normalize_or_zero();
                let ndoth = g.normal.dot(h).max(0.0);
                let gloss = (1.0 - g.roughness).max(0.02);
                let power = 2.0 + gloss * gloss * 126.0;
                let fresnel = schlick_fresnel(view.dot(h).max(0.0), material.class);
                let specular_ao = specular_occlusion(g.normal.dot(view).max(0.0), ao, g.roughness);
                specular +=
                    light_rgb * (ndoth.powf(power) * g.specular * shadow * specular_ao) * fresnel;
            }

            let opacity = if g.glass {
                g.opacity.clamp(0.04, 1.0)
            } else {
                1.0
            };
            let albedo = g.albedo.to_vec3();
            let transparent_diffuse_weight = if g.glass { 0.24 + opacity * 0.18 } else { 1.0 };
            let mut rgb = diffuse_irradiance * albedo * ao * transparent_diffuse_weight;
            rgb += specular * (0.45 + 0.55 * ao);
            rgb += buffers.reflections[idx].to_vec3() * (g.specular * (1.0 - g.roughness));
            if g.emissive {
                rgb += material.emission.to_vec3();
            }
            let luminance = rgb.x * 0.2126 + rgb.y * 0.7152 + rgb.z * 0.0722;
            buffers.lighting[idx] = [rgb.x, rgb.y, rgb.z, luminance];
        }
    }

    fn apply_fog(&self, scene: &RenderScene, buffers: &mut FrameBuffers) {
        for y in 0..self.config.height {
            for x in 0..self.config.width {
                let idx = self.index(x, y);
                let depth = buffers.depth[idx];
                if depth <= 0.0 {
                    continue;
                }
                let noise = (blue_noise_hash(x, y, self.config.frame_index, 0xF06B_100D) & 255)
                    as f32
                    / 255.0;
                let distance_depth = 1.0 - depth;
                let fog = saturate(distance_depth * distance_depth * 0.45 + noise * 0.015);
                let fog_rgb = scene.environment_radiance(camera_ray(
                    scene,
                    x,
                    y,
                    self.config.width,
                    self.config.height,
                ));
                buffers.lighting[idx][0] = buffers.lighting[idx][0] * (1.0 - fog) + fog_rgb.x * fog;
                buffers.lighting[idx][1] = buffers.lighting[idx][1] * (1.0 - fog) + fog_rgb.y * fog;
                buffers.lighting[idx][2] = buffers.lighting[idx][2] * (1.0 - fog) + fog_rgb.z * fog;
            }
        }
    }

    fn apply_bloom(&self, buffers: &mut FrameBuffers) -> Result<(), RenderError> {
        let thresholded: Vec<[f32; 4]> = buffers
            .lighting
            .iter()
            .map(|p| {
                let l = p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722;
                if l > 1.0 {
                    [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0, 1.0]
                } else {
                    [0.0; 4]
                }
            })
            .collect();
        let blurred = kawase_blur_rgba32f(&thresholded, self.config.width, self.config.height, 2)
            .map_err(|_| RenderError::KernelFailure)?;
        for (dst, bloom) in buffers.lighting.iter_mut().zip(blurred) {
            dst[0] += bloom[0] * 0.22;
            dst[1] += bloom[1] * 0.22;
            dst[2] += bloom[2] * 0.22;
        }
        Ok(())
    }

    fn apply_smaa(&self, buffers: &FrameBuffers, color: &mut [[u8; 4]]) {
        let tiles_w = self.config.width.div_ceil(4);
        for (tile_idx, edge) in buffers.edges.iter().enumerate() {
            if *edge == 0 {
                continue;
            }
            let tx = tile_idx as u32 % tiles_w;
            let ty = tile_idx as u32 / tiles_w;
            for oy in 0..4 {
                for ox in 0..4 {
                    let x = tx * 4 + ox;
                    let y = ty * 4 + oy;
                    if x + 1 >= self.config.width || y + 1 >= self.config.height {
                        continue;
                    }
                    let idx = self.index(x, y);
                    let right = self.index(x + 1, y);
                    let down = self.index(x, y + 1);
                    let right_pixel = color[right];
                    let down_pixel = color[down];
                    for (channel, value) in color[idx][..3].iter_mut().enumerate() {
                        *value = ((*value as u16 * 2
                            + right_pixel[channel] as u16
                            + down_pixel[channel] as u16)
                            / 4) as u8;
                    }
                }
            }
        }
    }

    fn apply_persistence(&self, lighting: &mut [[f32; 4]], edges: &[u8], gbuffer: &[GPixel]) {
        if self.history.len() != lighting.len() {
            return;
        }
        let tiles_w = self.config.width.div_ceil(4);
        for (tile_index, &edge) in edges.iter().enumerate() {
            if edge == 0 {
                continue;
            }
            let tile_x = tile_index as u32 % tiles_w;
            let tile_y = tile_index as u32 / tiles_w;
            for offset_y in 0..4 {
                for offset_x in 0..4 {
                    let x = tile_x * 4 + offset_x;
                    let y = tile_y * 4 + offset_y;
                    if x >= self.config.width || y >= self.config.height {
                        continue;
                    }
                    let index = self.index(x, y);
                    if gbuffer[index].responsive_aa {
                        continue;
                    }
                    let dst = &mut lighting[index];
                    let prev = self.history[index];
                    let delta = (dst[3] - prev[3]).abs();
                    if delta < 0.2 {
                        dst[0] = dst[0] * 0.84 + prev[0] * 0.16;
                        dst[1] = dst[1] * 0.84 + prev[1] * 0.16;
                        dst[2] = dst[2] * 0.84 + prev[2] * 0.16;
                    }
                }
            }
        }
    }

    fn debug_or_beauty(
        &self,
        scene: &RenderScene,
        buffers: &FrameBuffers,
        beauty: Vec<[u8; 4]>,
    ) -> Result<Vec<[u8; 4]>, RenderError> {
        if self.config.debug_view == DebugView::Beauty {
            return Ok(beauty);
        }
        let mut out = vec![[0, 0, 0, 255]; buffers.gbuffer.len()];
        for y in 0..self.config.height {
            for x in 0..self.config.width {
                let idx = self.index(x, y);
                let g = buffers.gbuffer[idx];
                let color = match self.config.debug_view {
                    DebugView::Beauty => unreachable!(),
                    DebugView::Depth => {
                        Color::rgb(buffers.depth[idx], buffers.depth[idx], buffers.depth[idx])
                    }
                    DebugView::Normals => Color::rgb(
                        g.normal.x * 0.5 + 0.5,
                        g.normal.y * 0.5 + 0.5,
                        g.normal.z * 0.5 + 0.5,
                    ),
                    DebugView::Roughness => Color::rgb(g.roughness, g.roughness, g.roughness),
                    DebugView::MaterialId => material_id_color(g.material_id),
                    DebugView::ShadowMask => Color::rgb(
                        buffers.shadow_mask[idx],
                        buffers.shadow_mask[idx],
                        buffers.shadow_mask[idx],
                    ),
                    DebugView::AmbientOcclusion => {
                        Color::rgb(buffers.ao[idx], buffers.ao[idx], buffers.ao[idx])
                    }
                    DebugView::MotionVectors => Color::rgb(
                        g.velocity.x.abs().min(1.0),
                        g.velocity.y.abs().min(1.0),
                        0.0,
                    ),
                    DebugView::Reflections => buffers.reflections[idx],
                    DebugView::EdgeStencil => {
                        let tiles_w = self.config.width.div_ceil(4);
                        let tile = (y / 4) * tiles_w + x / 4;
                        let edge = buffers
                            .edges
                            .get(tile as usize)
                            .copied()
                            .unwrap_or_default() as f32;
                        Color::rgb(edge, edge, edge)
                    }
                    DebugView::Overdraw => {
                        let v = (buffers.overdraw[idx] as f32 / 4.0).min(1.0);
                        Color::rgb(v, 0.1, 1.0 - v)
                    }
                    DebugView::TextureCount => {
                        let v = g.texture_count as f32
                            / scene
                                .materials
                                .get(g.material_index)
                                .map(|_| 4.0)
                                .unwrap_or(4.0);
                        Color::rgb(v.min(1.0), (1.0 - v).max(0.0), 0.0)
                    }
                    DebugView::BentNormalOcclusion => Color::rgb(
                        (g.normal.x * 0.5 + 0.5) * buffers.ao[idx],
                        (g.normal.y * 0.5 + 0.5) * buffers.ao[idx],
                        (g.normal.z * 0.5 + 0.5) * buffers.ao[idx],
                    ),
                    DebugView::QuadUtilization => {
                        let tiles_w = self.config.width.div_ceil(4);
                        let tile = (y / 4) * tiles_w + x / 4;
                        let edge = buffers
                            .edges
                            .get(tile as usize)
                            .copied()
                            .unwrap_or_default()
                            != 0;
                        if edge {
                            Color::rgb(1.0, 0.18, 0.05)
                        } else if g.covered {
                            Color::rgb(0.05, 0.75, 0.25)
                        } else {
                            Color::BLACK
                        }
                    }
                    DebugView::ShadowCasters => match g.shadow_class {
                        ShadowCasterClass::None => Color::rgb(0.12, 0.12, 0.12),
                        ShadowCasterClass::Main => Color::rgb(0.95, 0.25, 0.1),
                        ShadowCasterClass::ContactOnly => Color::rgb(1.0, 0.8, 0.1),
                        ShadowCasterClass::DistantOnly => Color::rgb(0.2, 0.5, 1.0),
                        ShadowCasterClass::FirstPerson => Color::rgb(0.8, 0.2, 1.0),
                        ShadowCasterClass::Proxy => Color::rgb(0.15, 0.9, 0.85),
                    },
                    DebugView::LightVolumes => {
                        let mut local = 0.0_f32;
                        for light in &scene.lights {
                            let local_light = match light.kind {
                                LightKind::Local { position, radius } => Some((position, radius)),
                                LightKind::Spot {
                                    position, radius, ..
                                } => Some((position, radius)),
                                LightKind::Directional { .. } => None,
                            };
                            if let Some((position, radius)) = local_light {
                                local = local.max(1.0 - (position - g.world).length() / radius);
                            }
                        }
                        Color::rgb(local, local * 0.35, 1.0 - local)
                    }
                    DebugView::HistoryValidity => {
                        let valid = self.history.get(idx).is_some_and(|previous| {
                            let current = buffers.lighting[idx];
                            (current[0] - previous[0]).abs()
                                + (current[1] - previous[1]).abs()
                                + (current[2] - previous[2]).abs()
                                < 0.12
                        });
                        if valid {
                            Color::rgb(0.05, 0.9, 0.2)
                        } else {
                            Color::rgb(0.95, 0.08, 0.05)
                        }
                    }
                };
                out[idx] = color.to_rgba8();
            }
        }
        Ok(out)
    }

    fn index(&self, x: u32, y: u32) -> usize {
        y as usize * self.config.width as usize + x as usize
    }
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn schlick_fresnel(cos_theta: f32, class: MaterialClass) -> Vec3 {
    let f0 = match class {
        MaterialClass::Metal => Vec3::new(1.0, 0.72, 0.28),
        MaterialClass::Glass => Vec3::splat(0.08),
        MaterialClass::Hair => Vec3::new(0.22, 0.18, 0.14),
        _ => Vec3::splat(0.04),
    };
    let f = (1.0 - cos_theta).powi(5);
    lerp3(f0, Vec3::ONE, f)
}

fn reflect(direction: Vec3, normal: Vec3) -> Vec3 {
    direction - normal * (2.0 * direction.dot(normal))
}

fn pow5(value: f32) -> f32 {
    let value2 = value * value;
    value2 * value2 * value
}

fn burley_diffuse(normal: Vec3, view: Vec3, light: Vec3, roughness: f32) -> f32 {
    let ndotl = normal.dot(light).max(0.0);
    let ndotv = normal.dot(view).max(0.0);
    let half_direction = (light + view).normalize_or_zero();
    let ldoth = light.dot(half_direction).max(0.0);
    let fd90 = 0.5 + 2.0 * ldoth * ldoth * roughness.max(0.04);
    let light_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - ndotl);
    let view_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - ndotv);
    ndotl * light_scatter * view_scatter * INV_PI
}

fn indirect_diffuse_irradiance(scene: &RenderScene, normal: Vec3) -> Vec3 {
    scene.environment_irradiance(normal)
}

fn camera_ray(scene: &RenderScene, x: u32, y: u32, width: u32, height: u32) -> Vec3 {
    let forward = (scene.camera.target - scene.camera.position).normalize_or_zero();
    let right = forward.cross(scene.camera.up).normalize_or_zero();
    let up = right.cross(forward).normalize_or_zero();
    let aspect = width as f32 / height.max(1) as f32;
    let tan_half_fov = (scene.camera.vertical_fov_radians * 0.5).tan();
    let ndc_x = ((x as f32 + 0.5) / width.max(1) as f32) * 2.0 - 1.0;
    let ndc_y = 1.0 - ((y as f32 + 0.5) / height.max(1) as f32) * 2.0;
    (forward + right * (ndc_x * aspect * tan_half_fov) + up * (ndc_y * tan_half_fov))
        .normalize_or_zero()
}

fn sky_luminance(color: Vec3) -> f32 {
    color.x * 0.2126 + color.y * 0.7152 + color.z * 0.0722
}

fn specular_occlusion(ndotv: f32, ao: f32, roughness: f32) -> f32 {
    ((ndotv + ao).powf(2.0_f32.powf(-16.0 * roughness.max(0.04) - 1.0)) - 1.0 + ao).clamp(0.0, 1.0)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0).max(1.0e-5)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn local_reflection_radiance(
    scene: &RenderScene,
    world: Vec3,
    reflected: Vec3,
    glass: bool,
) -> Color {
    let mut radiance = Vec3::ZERO;
    for light in &scene.lights {
        match light.kind {
            LightKind::Directional { direction } => {
                let light_direction = (-direction).normalize_or_zero();
                let alignment = reflected.dot(light_direction).max(0.0).powf(96.0);
                radiance += light.color.to_vec3() * (light.intensity * alignment * 0.45);
            }
            LightKind::Local { position, radius } => {
                let to_light = position - world;
                let distance_to_light = to_light.length();
                if distance_to_light >= radius {
                    continue;
                }
                let light_direction = to_light / distance_to_light.max(1.0e-4);
                let attenuation = (1.0 - distance_to_light / radius).powi(2);
                let alignment = reflected.dot(light_direction).max(0.0);
                let glossy_lobe = alignment.powf(if glass { 5.0 } else { 13.0 });
                let ground_streak = ground_reflection_streak(world, position);
                let water_streak = water_fixture_streak(world, position, distance_to_light, glass);
                radiance += light.color.to_vec3()
                    * (light.intensity
                        * (attenuation + water_streak)
                        * ground_streak
                        * (0.04 + glossy_lobe * if glass { 1.85 } else { 0.92 }));
            }
            LightKind::Spot {
                position,
                direction,
                radius,
                inner_angle_radians,
                outer_angle_radians,
            } => {
                let to_light = position - world;
                let distance_to_light = to_light.length();
                if distance_to_light >= radius {
                    continue;
                }
                let light_direction = to_light / distance_to_light.max(1.0e-4);
                let light_to_surface = -light_direction;
                let inner = inner_angle_radians.cos();
                let outer = outer_angle_radians.cos();
                let cone = smoothstep(
                    outer,
                    inner,
                    light_to_surface.dot(direction.normalize_or_zero()),
                );
                let halo = smoothstep(
                    outer - 0.20,
                    outer + 0.015,
                    light_to_surface.dot(direction.normalize_or_zero()),
                ) * 0.24;
                let spot = cone.max(halo * (1.0 - cone * 0.45));
                if spot <= 0.001 {
                    continue;
                }
                let attenuation = (1.0 - distance_to_light / radius).powi(2);
                let alignment = reflected.dot(light_direction).max(0.0);
                let glossy_lobe = alignment.powf(if glass { 5.0 } else { 13.0 });
                let ground_streak = ground_reflection_streak(world, position);
                let water_streak = water_fixture_streak(world, position, distance_to_light, glass);
                radiance += light.color.to_vec3()
                    * (light.intensity
                        * (attenuation + water_streak)
                        * spot
                        * ground_streak
                        * (0.05 + glossy_lobe * if glass { 2.15 } else { 1.05 }));
            }
        }
    }
    Color::from_vec3(radiance, 1.0)
}

fn ground_reflection_streak(world: Vec3, light_position: Vec3) -> f32 {
    if world.y > 0.12 {
        return 1.0;
    }
    let lateral = (world.x - light_position.x).abs();
    let depth = (world.z - light_position.z).abs();
    1.0 / (1.0 + lateral * 0.13 + depth * 0.038)
}

fn transparent_underlay(world: Vec3) -> Vec3 {
    let asphalt = Vec3::new(0.018, 0.022, 0.025);
    let concrete = Vec3::new(0.060, 0.058, 0.052);
    let walkway = smoothstep(3.45, 4.75, world.x.abs());
    let base = lerp3(asphalt, concrete, walkway);
    let stain = ((world.x * 11.7 + world.z * 5.3).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
    base * (0.72 + stain * 0.36)
}

fn water_fixture_streak(
    world: Vec3,
    light_position: Vec3,
    distance_to_light: f32,
    glass: bool,
) -> f32 {
    if !glass || world.y > 0.12 {
        return 0.0;
    }
    let lateral = (world.x - light_position.x).abs();
    let longitudinal = (world.z - light_position.z).abs();
    let lane = 1.0 / (1.0 + lateral * lateral * 0.10);
    let length_fade = 1.0 / (1.0 + longitudinal * 0.045 + distance_to_light * 0.055);
    lane * length_fade * 0.34
}

fn trace_alley_proxy(scene: &RenderScene, origin: Vec3, ray: Vec3) -> Color {
    let mut best_t = 1_000_000.0_f32;
    let mut best_radiance = Vec3::ZERO;

    if ray.y.abs() > 0.0001 {
        let t = (-0.06 - origin.y) / ray.y;
        let p = origin + ray * t;
        if t > 0.04 && t < best_t && p.x.abs() < 18.5 && p.z > -82.0 && p.z < 14.0 {
            best_t = t;
            best_radiance =
                proxy_direct_radiance(scene, p, Vec3::Y, Vec3::new(0.045, 0.052, 0.055));
        }
    }

    if ray.x.abs() > 0.0001 {
        let left_t = (-12.5 - origin.x) / ray.x;
        let left_p = origin + ray * left_t;
        if left_t > 0.04
            && left_t < best_t
            && left_p.y > 0.0
            && left_p.y < 16.0
            && left_p.z > -82.0
            && left_p.z < 14.0
        {
            best_t = left_t;
            best_radiance =
                proxy_direct_radiance(scene, left_p, Vec3::X, Vec3::new(0.18, 0.125, 0.10));
        }

        let right_t = (12.5 - origin.x) / ray.x;
        let right_p = origin + ray * right_t;
        if right_t > 0.04
            && right_t < best_t
            && right_p.y > 0.0
            && right_p.y < 16.0
            && right_p.z > -82.0
            && right_p.z < 14.0
        {
            best_t = right_t;
            best_radiance =
                proxy_direct_radiance(scene, right_p, -Vec3::X, Vec3::new(0.18, 0.125, 0.10));
        }
    }

    if ray.z.abs() > 0.0001 {
        let end_t = (-78.5 - origin.z) / ray.z;
        let end_p = origin + ray * end_t;
        if end_t > 0.04 && end_t < best_t && end_p.x.abs() < 18.5 && end_p.y > 0.0 && end_p.y < 18.0
        {
            best_t = end_t;
            best_radiance =
                proxy_direct_radiance(scene, end_p, Vec3::Z, Vec3::new(0.17, 0.12, 0.10));
        }
    }

    if best_t >= 1_000_000.0 {
        return Color::BLACK;
    }
    let distance_fade = 1.0 / (1.0 + best_t * 0.025);
    Color::from_vec3(best_radiance * distance_fade, 1.0)
}

fn proxy_direct_radiance(scene: &RenderScene, position: Vec3, normal: Vec3, albedo: Vec3) -> Vec3 {
    let mut radiance = albedo * indirect_diffuse_irradiance(scene, normal) * INV_PI;
    for light in &scene.lights {
        match light.kind {
            LightKind::Directional { direction } => {
                let light_direction = (-direction).normalize_or_zero();
                radiance += albedo
                    * light.color.to_vec3()
                    * (light.intensity * light_direction.dot(normal).max(0.0) * 0.55 * INV_PI);
            }
            LightKind::Local {
                position: light_position,
                radius,
            } => {
                let to_light = light_position - position;
                let distance_to_light = to_light.length();
                if distance_to_light >= radius {
                    continue;
                }
                let light_direction = to_light / distance_to_light.max(1.0e-4);
                let attenuation = (1.0 - distance_to_light / radius).powi(2);
                radiance += albedo
                    * light.color.to_vec3()
                    * (light.intensity
                        * attenuation
                        * light_direction.dot(normal).max(0.0)
                        * 0.72
                        * INV_PI);
            }
            LightKind::Spot {
                position: light_position,
                direction,
                radius,
                inner_angle_radians,
                outer_angle_radians,
            } => {
                let to_light = light_position - position;
                let distance_to_light = to_light.length();
                if distance_to_light >= radius {
                    continue;
                }
                let light_direction = to_light / distance_to_light.max(1.0e-4);
                let light_to_surface = -light_direction;
                let inner = inner_angle_radians.cos();
                let outer = outer_angle_radians.cos();
                let cone = saturate(
                    (light_to_surface.dot(direction.normalize_or_zero()) - outer)
                        / (inner - outer).max(1.0e-4),
                );
                let smooth_cone = cone * cone * (3.0 - 2.0 * cone);
                let practical_fill = 0.42 + smooth_cone * 0.58;
                let attenuation = (1.0 - distance_to_light / radius).powi(2);
                radiance += albedo
                    * light.color.to_vec3()
                    * (light.intensity
                        * attenuation
                        * practical_fill
                        * light_direction.dot(normal).max(0.0)
                        * 0.72
                        * INV_PI);
            }
        }
    }
    radiance + proxy_emissive_radiance(position)
}

fn proxy_emissive_radiance(position: Vec3) -> Vec3 {
    let mut emission = Vec3::ZERO;
    if (position.x - 4.88).abs() < 0.20 && (position.z + 6.7).abs() < 1.38 {
        let smear = 1.0 - smoothstep(0.18, 2.10, (position.y - 2.65).abs());
        emission += Vec3::new(9.5, 2.2, 0.28) * smear;
    }
    if (position.x + 4.88).abs() < 0.20 && (position.z + 5.4).abs() < 1.78 {
        let smear = 1.0 - smoothstep(0.18, 2.25, (position.y - 2.8).abs());
        emission += Vec3::new(0.65, 6.2, 9.5) * smear;
    }
    if (position.z + 18.38).abs() < 0.22 && (position.x + 1.7).abs() < 2.08 {
        let smear = 1.0 - smoothstep(0.22, 2.65, (position.y - 3.25).abs());
        emission += Vec3::new(0.55, 5.8, 8.8) * smear;
    }
    emission
}

fn material_id_color(id: u8) -> Color {
    let h = (id as u32)
        .wrapping_mul(747_796_405u32)
        .wrapping_add(2_891_336_453u32);
    let r = ((h >> 16) & 255) as f32 / 255.0;
    let g = ((h >> 8) & 255) as f32 / 255.0;
    let b = (h & 255) as f32 / 255.0;
    Color::rgb(r, g, b)
}
