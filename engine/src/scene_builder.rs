use crate::asset::ShadowCasterClass;
use crate::math::{Color, Mat4, Vec2, Vec3};
use crate::scene::{
    Camera, EnvironmentMap, EnvironmentProfile, Light, LightKind, Material, MaterialClass,
    MaterialFlags, Mesh, RenderScene, Vertex,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialId(usize);

impl MaterialId {
    pub const fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneBuildError {
    InvalidCameraClipPlanes,
    TooManyMaterials { count: usize },
    EmptyMaterialName { index: usize },
    EmptyMeshName { index: usize },
    MissingMaterial { mesh: String, material: usize },
    EmptyMesh { mesh: String },
}

impl fmt::Display for SceneBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCameraClipPlanes => {
                write!(
                    formatter,
                    "camera near plane must be positive and less than far"
                )
            }
            Self::TooManyMaterials { count } => {
                write!(
                    formatter,
                    "a scene supports at most 256 materials, got {count}"
                )
            }
            Self::EmptyMaterialName { index } => {
                write!(formatter, "material {index} has an empty name")
            }
            Self::EmptyMeshName { index } => write!(formatter, "mesh {index} has an empty name"),
            Self::MissingMaterial { mesh, material } => {
                write!(
                    formatter,
                    "mesh '{mesh}' references missing material {material}"
                )
            }
            Self::EmptyMesh { mesh } => write!(formatter, "mesh '{mesh}' has no geometry"),
        }
    }
}

impl std::error::Error for SceneBuildError {}

/// Builds a renderer-ready scene without relying on bundled project content.
pub struct SceneBuilder {
    scene: RenderScene,
}

impl Default for SceneBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneBuilder {
    pub fn new() -> Self {
        Self {
            scene: RenderScene {
                camera: Camera {
                    position: Vec3::new(0.0, 2.0, 6.0),
                    target: Vec3::ZERO,
                    up: Vec3::Y,
                    vertical_fov_radians: 60.0_f32.to_radians(),
                    near: 0.05,
                    far: 250.0,
                },
                materials: Vec::new(),
                meshes: Vec::new(),
                lights: Vec::new(),
                ambient_color: Color::rgb(0.03, 0.04, 0.06),
                environment_map: EnvironmentMap::solid(Color::rgb(0.10, 0.14, 0.20)),
                environment: EnvironmentProfile::dry_daylight(),
            },
        }
    }

    pub fn camera(mut self, camera: Camera) -> Self {
        self.scene.camera = camera;
        self
    }

    pub fn environment(
        mut self,
        map: EnvironmentMap,
        profile: EnvironmentProfile,
        ambient_color: Color,
    ) -> Self {
        self.scene.environment_map = map;
        self.scene.environment = profile;
        self.scene.ambient_color = ambient_color;
        self
    }

    pub fn add_material(&mut self, mut material: Material) -> MaterialId {
        let id = MaterialId(self.scene.materials.len());
        material.material_id = id.0 as u8;
        self.scene.materials.push(material);
        id
    }

    pub fn add_mesh(&mut self, mut mesh: Mesh, material: MaterialId) {
        mesh.material_index = material.index();
        self.scene.meshes.push(mesh);
    }

    pub fn add_light(&mut self, light: Light) {
        self.scene.lights.push(light);
    }

    pub fn build(self) -> Result<RenderScene, SceneBuildError> {
        if self.scene.camera.near <= 0.0 || self.scene.camera.far <= self.scene.camera.near {
            return Err(SceneBuildError::InvalidCameraClipPlanes);
        }
        if self.scene.materials.len() > u8::MAX as usize + 1 {
            return Err(SceneBuildError::TooManyMaterials {
                count: self.scene.materials.len(),
            });
        }
        for (index, material) in self.scene.materials.iter().enumerate() {
            if material.name.trim().is_empty() {
                return Err(SceneBuildError::EmptyMaterialName { index });
            }
        }
        for (index, mesh) in self.scene.meshes.iter().enumerate() {
            if mesh.name.trim().is_empty() {
                return Err(SceneBuildError::EmptyMeshName { index });
            }
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                return Err(SceneBuildError::EmptyMesh {
                    mesh: mesh.name.clone(),
                });
            }
            if mesh.material_index >= self.scene.materials.len() {
                return Err(SceneBuildError::MissingMaterial {
                    mesh: mesh.name.clone(),
                    material: mesh.material_index,
                });
            }
        }
        Ok(self.scene)
    }
}

impl Material {
    pub fn standard(name: impl Into<String>, base_color: Color) -> Self {
        Self {
            name: name.into(),
            class: MaterialClass::Default,
            base_color,
            base_color_texture: None,
            normal_texture: None,
            pbr_texture: None,
            roughness: 0.65,
            metallic: 0.0,
            specular_intensity: 0.12,
            cavity: 0.0,
            material_id: 0,
            texture_sample_count: 0,
            flags: MaterialFlags::default(),
            emission: Color::BLACK,
        }
    }
}

impl Light {
    pub fn directional(
        name: impl Into<String>,
        direction: Vec3,
        color: Color,
        intensity: f32,
    ) -> Self {
        Self {
            name: name.into(),
            kind: LightKind::Directional { direction },
            color,
            intensity,
            casts_shadow: true,
        }
    }

    pub fn point(
        name: impl Into<String>,
        position: Vec3,
        radius: f32,
        color: Color,
        intensity: f32,
    ) -> Self {
        Self {
            name: name.into(),
            kind: LightKind::Local { position, radius },
            color,
            intensity,
            casts_shadow: false,
        }
    }
}

impl Mesh {
    pub fn cuboid(name: impl Into<String>, size: Vec3) -> Self {
        let half = size * 0.5;
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(12);
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(-half.x, -half.y, half.z),
                Vec3::new(half.x, -half.y, half.z),
                Vec3::new(half.x, half.y, half.z),
                Vec3::new(-half.x, half.y, half.z),
            ],
            Vec3::Z,
        );
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(half.x, -half.y, -half.z),
                Vec3::new(-half.x, -half.y, -half.z),
                Vec3::new(-half.x, half.y, -half.z),
                Vec3::new(half.x, half.y, -half.z),
            ],
            -Vec3::Z,
        );
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(half.x, -half.y, half.z),
                Vec3::new(half.x, -half.y, -half.z),
                Vec3::new(half.x, half.y, -half.z),
                Vec3::new(half.x, half.y, half.z),
            ],
            Vec3::X,
        );
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(-half.x, -half.y, -half.z),
                Vec3::new(-half.x, -half.y, half.z),
                Vec3::new(-half.x, half.y, half.z),
                Vec3::new(-half.x, half.y, -half.z),
            ],
            -Vec3::X,
        );
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(-half.x, half.y, half.z),
                Vec3::new(half.x, half.y, half.z),
                Vec3::new(half.x, half.y, -half.z),
                Vec3::new(-half.x, half.y, -half.z),
            ],
            Vec3::Y,
        );
        push_face(
            &mut vertices,
            &mut indices,
            [
                Vec3::new(-half.x, -half.y, -half.z),
                Vec3::new(half.x, -half.y, -half.z),
                Vec3::new(half.x, -half.y, half.z),
                Vec3::new(-half.x, -half.y, half.z),
            ],
            -Vec3::Y,
        );
        Self {
            name: name.into(),
            vertices,
            indices,
            material_index: 0,
            transform: Mat4::IDENTITY,
            shadow_class: ShadowCasterClass::Main,
            is_floor_or_ground: false,
            prepass_eligible: true,
            reflection_proxy: false,
            cloth: None,
        }
    }

    pub fn ground_plane(name: impl Into<String>, size: Vec2) -> Self {
        let half = size * 0.5;
        let positions = [
            Vec3::new(-half.x, 0.0, -half.y),
            Vec3::new(-half.x, 0.0, half.y),
            Vec3::new(half.x, 0.0, half.y),
            Vec3::new(half.x, 0.0, -half.y),
        ];
        let mut vertices = Vec::with_capacity(4);
        let mut indices = Vec::with_capacity(2);
        push_face(&mut vertices, &mut indices, positions, Vec3::Y);
        Self {
            name: name.into(),
            vertices,
            indices,
            material_index: 0,
            transform: Mat4::IDENTITY,
            shadow_class: ShadowCasterClass::None,
            is_floor_or_ground: true,
            prepass_eligible: true,
            reflection_proxy: false,
            cloth: None,
        }
    }
}

fn push_face(
    vertices: &mut Vec<Vertex>,
    indices: &mut Vec<[u32; 3]>,
    positions: [Vec3; 4],
    normal: Vec3,
) {
    let start = vertices.len() as u32;
    let tangent = (positions[1] - positions[0]).normalize_or_zero();
    let uvs = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(0.0, 1.0),
    ];
    vertices.extend(positions.into_iter().zip(uvs).map(|(position, uv)| Vertex {
        position,
        previous_position: position,
        normal,
        tangent,
        tangent_sign: 1.0,
        uv,
        color: Color::WHITE,
    }));
    indices.push([start, start + 1, start + 2]);
    indices.push([start, start + 2, start + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_scene_has_no_demo_asset_dependency() {
        let mut builder = SceneBuilder::new();
        let material = builder.add_material(Material::standard("cube", Color::rgb(0.7, 0.2, 0.1)));
        builder.add_mesh(Mesh::cuboid("player", Vec3::ONE), material);
        builder.add_light(Light::directional(
            "sun",
            Vec3::new(0.2, -1.0, -0.3),
            Color::WHITE,
            2.0,
        ));

        let scene = builder.build().expect("custom scene");
        assert_eq!(scene.materials.len(), 1);
        assert_eq!(scene.meshes[0].vertices.len(), 24);
        assert_eq!(scene.meshes[0].indices.len(), 12);
        assert_eq!(scene.lights[0].name, "sun");
    }
}
