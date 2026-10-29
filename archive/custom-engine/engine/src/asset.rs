use crate::quality::QualityTier;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Bc1,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
    Rgba8,
    Rgba16Float,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureRole {
    NormalsAndBentNormals,
    BaseColorAndAo,
    SpecularRoughnessHeight,
    BinaryMask,
    Opacity,
    EnvironmentRadiance,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureDesc {
    pub name: String,
    pub format: TextureFormat,
    pub role: TextureRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialDesc {
    pub name: String,
    pub textures: Vec<TextureDesc>,
    pub texture_sample_count: usize,
    pub runtime_texture_blending: bool,
    pub requires_specular_occlusion: bool,
    pub has_bent_normals: bool,
    pub alpha_tested: bool,
}

impl MaterialDesc {
    pub fn opacity_format(&self) -> Option<TextureFormat> {
        self.textures
            .iter()
            .find(|texture| texture.role == TextureRole::Opacity)
            .map(|texture| texture.format)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowCasterClass {
    Main,
    ContactOnly,
    DistantOnly,
    FirstPerson,
    None,
    Proxy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshDesc {
    pub name: String,
    pub material_index: usize,
    pub is_floor_or_ground: bool,
    pub shadow_class: ShadowCasterClass,
    pub participates_in_prepass: bool,
    pub writes_motion_vectors_in_prepass: bool,
    pub long_thin_triangles: bool,
    pub dense_alpha_cards: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetIssue {
    TooManyMaterialTextures {
        material: String,
        count: usize,
        limit: usize,
    },
    Bc5NormalMap {
        material: String,
        texture: String,
    },
    MissingBentNormals {
        material: String,
    },
    RuntimeTextureBlending {
        material: String,
    },
    MissingSeparateBc4Opacity {
        material: String,
    },
    FloorCastsShadow {
        mesh: String,
    },
    AlphaTestedInPrepass {
        mesh: String,
        material: String,
    },
    MotionVectorsInPrepass {
        mesh: String,
    },
    DenseAlphaCards {
        mesh: String,
    },
    LongThinTriangles {
        mesh: String,
    },
    InvalidMaterialIndex {
        mesh: String,
        index: usize,
    },
}

pub fn validate_material(material: &MaterialDesc, tier: QualityTier) -> Vec<AssetIssue> {
    let mut issues = Vec::new();
    let limit = tier.material_texture_limit();

    if material.texture_sample_count > limit {
        issues.push(AssetIssue::TooManyMaterialTextures {
            material: material.name.clone(),
            count: material.texture_sample_count,
            limit,
        });
    }

    for texture in &material.textures {
        if texture.role == TextureRole::NormalsAndBentNormals
            && texture.format == TextureFormat::Bc5
        {
            issues.push(AssetIssue::Bc5NormalMap {
                material: material.name.clone(),
                texture: texture.name.clone(),
            });
        }
    }

    if material.requires_specular_occlusion && !material.has_bent_normals {
        issues.push(AssetIssue::MissingBentNormals {
            material: material.name.clone(),
        });
    }

    if material.runtime_texture_blending {
        issues.push(AssetIssue::RuntimeTextureBlending {
            material: material.name.clone(),
        });
    }

    if material.alpha_tested && material.opacity_format() != Some(TextureFormat::Bc4) {
        issues.push(AssetIssue::MissingSeparateBc4Opacity {
            material: material.name.clone(),
        });
    }

    issues
}

pub fn validate_mesh(mesh: &MeshDesc, material: &MaterialDesc) -> Vec<AssetIssue> {
    let mut issues = Vec::new();

    if mesh.is_floor_or_ground && mesh.shadow_class != ShadowCasterClass::None {
        issues.push(AssetIssue::FloorCastsShadow {
            mesh: mesh.name.clone(),
        });
    }

    if mesh.participates_in_prepass && material.alpha_tested {
        issues.push(AssetIssue::AlphaTestedInPrepass {
            mesh: mesh.name.clone(),
            material: material.name.clone(),
        });
    }

    if mesh.writes_motion_vectors_in_prepass {
        issues.push(AssetIssue::MotionVectorsInPrepass {
            mesh: mesh.name.clone(),
        });
    }

    if mesh.dense_alpha_cards {
        issues.push(AssetIssue::DenseAlphaCards {
            mesh: mesh.name.clone(),
        });
    }

    if mesh.long_thin_triangles {
        issues.push(AssetIssue::LongThinTriangles {
            mesh: mesh.name.clone(),
        });
    }

    issues
}

pub fn validate_asset(
    materials: &[MaterialDesc],
    meshes: &[MeshDesc],
    tier: QualityTier,
) -> Vec<AssetIssue> {
    let mut issues = Vec::new();

    for material in materials {
        issues.extend(validate_material(material, tier));
    }

    for mesh in meshes {
        match materials.get(mesh.material_index) {
            Some(material) => issues.extend(validate_mesh(mesh, material)),
            None => issues.push(AssetIssue::InvalidMaterialIndex {
                mesh: mesh.name.clone(),
                index: mesh.material_index,
            }),
        }
    }

    issues
}
