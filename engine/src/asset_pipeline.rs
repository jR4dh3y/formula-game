use crate::asset::{
    validate_asset, AssetIssue, MaterialDesc, MeshDesc, TextureDesc, TextureFormat, TextureRole,
};
use crate::quality::QualityTier;
use crate::scene::{MaterialClass, RenderScene};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssetId(u64);

impl AssetId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    Material,
    Mesh,
    Texture,
    Scene,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImportedAsset {
    Material(MaterialDesc),
    Mesh(MeshDesc),
    Texture(TextureDesc),
    SceneSummary {
        materials: usize,
        meshes: usize,
        lights: usize,
    },
}

impl ImportedAsset {
    pub const fn kind(&self) -> AssetKind {
        match self {
            Self::Material(_) => AssetKind::Material,
            Self::Mesh(_) => AssetKind::Mesh,
            Self::Texture(_) => AssetKind::Texture,
            Self::SceneSummary { .. } => AssetKind::Scene,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportMessage {
    pub severity: ImportSeverity,
    pub asset: Option<AssetId>,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub messages: Vec<ImportMessage>,
}

impl ImportReport {
    pub fn push(
        &mut self,
        severity: ImportSeverity,
        asset: Option<AssetId>,
        text: impl Into<String>,
    ) {
        self.messages.push(ImportMessage {
            severity,
            asset,
            text: text.into(),
        });
    }

    pub fn has_errors(&self) -> bool {
        self.messages
            .iter()
            .any(|m| m.severity == ImportSeverity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &ImportMessage> {
        self.messages
            .iter()
            .filter(|m| m.severity == ImportSeverity::Error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TexturePackingPolicy {
    pub normal_format: TextureFormat,
    pub base_format: TextureFormat,
    pub specular_format: TextureFormat,
    pub opacity_format: TextureFormat,
    pub allow_bc5_normals: bool,
}

impl TexturePackingPolicy {
    pub const GOSPOL_BASELINE: Self = Self {
        normal_format: TextureFormat::Bc7,
        base_format: TextureFormat::Bc3,
        specular_format: TextureFormat::Bc3,
        opacity_format: TextureFormat::Bc4,
        allow_bc5_normals: false,
    };

    pub fn validate_texture(&self, texture: &TextureDesc) -> Option<String> {
        match (texture.role, texture.format) {
            (TextureRole::NormalsAndBentNormals, TextureFormat::Bc5) if !self.allow_bc5_normals => {
                Some(format!(
                    "{} uses BC5 normals; normals+bent normals must be BC7",
                    texture.name
                ))
            }
            (TextureRole::Opacity, fmt) if fmt != self.opacity_format => Some(format!(
                "{} opacity must use a standalone BC4 texture",
                texture.name
            )),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LodPolicy {
    pub require_shadow_lods: bool,
    pub require_reflection_lods: bool,
    pub max_transition_error: f32,
}

impl Default for LodPolicy {
    fn default() -> Self {
        Self {
            require_shadow_lods: true,
            require_reflection_lods: true,
            max_transition_error: 0.08,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportedRecord {
    pub id: AssetId,
    pub name: String,
    pub asset: ImportedAsset,
}

#[derive(Clone, Debug)]
pub struct AssetPipeline {
    next_id: u64,
    records: Vec<ImportedRecord>,
    name_to_id: HashMap<String, AssetId>,
    pub texture_policy: TexturePackingPolicy,
    pub lod_policy: LodPolicy,
    pub quality: QualityTier,
}

impl AssetPipeline {
    pub fn new(quality: QualityTier) -> Self {
        Self {
            next_id: 1,
            records: Vec::new(),
            name_to_id: HashMap::new(),
            texture_policy: TexturePackingPolicy::GOSPOL_BASELINE,
            lod_policy: LodPolicy::default(),
            quality,
        }
    }

    pub fn import_asset(&mut self, name: impl Into<String>, asset: ImportedAsset) -> AssetId {
        let name = name.into();
        let id = AssetId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.name_to_id.insert(name.clone(), id);
        self.records.push(ImportedRecord { id, name, asset });
        id
    }

    pub fn asset(&self, id: AssetId) -> Option<&ImportedRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    pub fn find(&self, name: &str) -> Option<AssetId> {
        self.name_to_id.get(name).copied()
    }

    pub fn records(&self) -> &[ImportedRecord] {
        &self.records
    }

    pub fn validate(&self) -> ImportReport {
        let mut report = ImportReport::default();
        let mut materials = Vec::new();
        let mut meshes = Vec::new();
        let mut material_ids = Vec::new();
        let mut mesh_ids = Vec::new();

        for record in &self.records {
            match &record.asset {
                ImportedAsset::Material(material) => {
                    for texture in &material.textures {
                        if let Some(msg) = self.texture_policy.validate_texture(texture) {
                            report.push(ImportSeverity::Error, Some(record.id), msg);
                        }
                    }
                    if material.texture_sample_count > self.quality.material_texture_limit() {
                        report.push(
                            ImportSeverity::Error,
                            Some(record.id),
                            format!(
                                "{} uses {} textures; tier limit is {}",
                                material.name,
                                material.texture_sample_count,
                                self.quality.material_texture_limit()
                            ),
                        );
                    }
                    materials.push(material.clone());
                    material_ids.push(record.id);
                }
                ImportedAsset::Mesh(mesh) => {
                    meshes.push(mesh.clone());
                    mesh_ids.push(record.id);
                }
                ImportedAsset::Texture(texture) => {
                    if let Some(msg) = self.texture_policy.validate_texture(texture) {
                        report.push(ImportSeverity::Error, Some(record.id), msg);
                    }
                }
                ImportedAsset::SceneSummary {
                    materials,
                    meshes,
                    lights,
                } => {
                    report.push(
                        ImportSeverity::Info,
                        Some(record.id),
                        format!("scene summary: {materials} materials, {meshes} meshes, {lights} lights"),
                    );
                }
            }
        }

        for issue in validate_asset(&materials, &meshes, self.quality) {
            let (asset, text) = match issue {
                AssetIssue::TooManyMaterialTextures {
                    material,
                    count,
                    limit,
                } => (
                    find_material_id(&materials, &material, &material_ids),
                    format!("{material} uses {count} texture samples; limit is {limit}"),
                ),
                AssetIssue::Bc5NormalMap { material, texture } => (
                    find_material_id(&materials, &material, &material_ids),
                    format!("{material} uses prohibited BC5 normal map {texture}"),
                ),
                AssetIssue::MissingBentNormals { material } => (
                    find_material_id(&materials, &material, &material_ids),
                    format!("{material} needs bent normals for specular occlusion"),
                ),
                AssetIssue::RuntimeTextureBlending { material } => (
                    find_material_id(&materials, &material, &material_ids),
                    format!("{material} uses prohibited runtime texture blending"),
                ),
                AssetIssue::MissingSeparateBc4Opacity { material } => (
                    find_material_id(&materials, &material, &material_ids),
                    format!("{material} needs separate BC4 opacity for alpha-tested passes"),
                ),
                AssetIssue::FloorCastsShadow { mesh } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh} is floor/ground and must not cast shadows"),
                ),
                AssetIssue::AlphaTestedInPrepass { mesh, material } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh}/{material} is alpha-tested and must not enter the prepass"),
                ),
                AssetIssue::MotionVectorsInPrepass { mesh } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh} computes motion vectors in prepass; base pass only"),
                ),
                AssetIssue::DenseAlphaCards { mesh } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh} has dense alpha cards"),
                ),
                AssetIssue::LongThinTriangles { mesh } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh} has long thin triangles hurting quad utilization"),
                ),
                AssetIssue::InvalidMaterialIndex { mesh, index } => (
                    find_mesh_id(&meshes, &mesh, &mesh_ids),
                    format!("{mesh} references invalid material index {index}"),
                ),
            };
            report.push(ImportSeverity::Error, asset, text);
        }
        report
    }

    pub fn build_from_scene(scene: &RenderScene, quality: QualityTier) -> (Self, ImportReport) {
        let mut pipeline = Self::new(quality);
        let environment_id = pipeline.import_asset(
            "scene_environment",
            ImportedAsset::Texture(TextureDesc {
                name: "scene_environment_radiance".to_string(),
                format: TextureFormat::Rgba16Float,
                role: TextureRole::EnvironmentRadiance,
            }),
        );
        let environment_error = validate_environment_dimensions(scene);
        for material in &scene.materials {
            pipeline.import_asset(
                material.name.clone(),
                ImportedAsset::Material(material.to_asset_desc()),
            );
        }
        for mesh in &scene.meshes {
            pipeline.import_asset(mesh.name.clone(), ImportedAsset::Mesh(mesh.to_asset_desc()));
            if let Some(material) = scene.materials.get(mesh.material_index) {
                let reflective =
                    matches!(material.class, MaterialClass::Glass | MaterialClass::Metal)
                        || material.specular_intensity > 0.75;
                if reflective && !mesh.reflection_proxy {
                    let id = pipeline.find(&mesh.name);
                    let mut report = pipeline.validate();
                    if let Some(message) = &environment_error {
                        report.push(ImportSeverity::Error, Some(environment_id), message.clone());
                    }
                    report.push(
                        ImportSeverity::Error,
                        id,
                        format!("{} is reflective and needs a reflection proxy", mesh.name),
                    );
                    pipeline.import_asset(
                        "scene_summary",
                        ImportedAsset::SceneSummary {
                            materials: scene.materials.len(),
                            meshes: scene.meshes.len(),
                            lights: scene.lights.len(),
                        },
                    );
                    return (pipeline, report);
                }
            }
        }
        pipeline.import_asset(
            "scene_summary",
            ImportedAsset::SceneSummary {
                materials: scene.materials.len(),
                meshes: scene.meshes.len(),
                lights: scene.lights.len(),
            },
        );
        let mut report = pipeline.validate();
        if let Some(message) = environment_error {
            report.push(ImportSeverity::Error, Some(environment_id), message);
        }
        (pipeline, report)
    }
}

fn validate_environment_dimensions(scene: &RenderScene) -> Option<String> {
    let panorama = &scene.environment_map.panorama;
    if panorama.width == 0 || panorama.height == 0 || panorama.pixels.is_empty() {
        return Some("scene environment panorama is empty".to_string());
    }
    if panorama.width != panorama.height.saturating_mul(2) {
        return Some(format!(
            "scene environment panorama must be 2:1 equirectangular, got {}x{}",
            panorama.width, panorama.height
        ));
    }
    if panorama.pixels.len() != panorama.width.saturating_mul(panorama.height) as usize {
        return Some(format!(
            "scene environment panorama has {} pixels, expected {}",
            panorama.pixels.len(),
            panorama.width.saturating_mul(panorama.height)
        ));
    }
    None
}

fn find_material_id(materials: &[MaterialDesc], name: &str, ids: &[AssetId]) -> Option<AssetId> {
    materials
        .iter()
        .position(|m| m.name == name)
        .and_then(|i| ids.get(i).copied())
}

fn find_mesh_id(meshes: &[MeshDesc], name: &str, ids: &[AssetId]) -> Option<AssetId> {
    meshes
        .iter()
        .position(|m| m.name == name)
        .and_then(|i| ids.get(i).copied())
}
