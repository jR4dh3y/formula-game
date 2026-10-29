use crate::asset::ShadowCasterClass;
use crate::math::{Color, Mat4, Vec2, Vec3};
use crate::scene::{Material, MaterialClass, MaterialFlags, Mesh, TextureImage, Vertex};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::{error::Error, fmt};

#[derive(Clone, Debug, PartialEq)]
pub enum GltfImportError {
    Gltf(String),
    Image(String),
    MissingPositions {
        mesh: String,
    },
    InvalidIndexCount {
        mesh: String,
        count: usize,
    },
    InvalidIndexValue {
        mesh: String,
        index: u32,
        vertex_count: usize,
    },
}

impl fmt::Display for GltfImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gltf(message) => write!(formatter, "glTF import failed: {message}"),
            Self::Image(message) => write!(formatter, "glTF image decode failed: {message}"),
            Self::MissingPositions { mesh } => write!(formatter, "{mesh} has no positions"),
            Self::InvalidIndexCount { mesh, count } => {
                write!(formatter, "{mesh} has {count} indices, not a triangle list")
            }
            Self::InvalidIndexValue {
                mesh,
                index,
                vertex_count,
            } => write!(
                formatter,
                "{mesh} references vertex index {index}, but only has {vertex_count} vertices"
            ),
        }
    }
}

impl Error for GltfImportError {}

/// Controls how much texture data an import keeps in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfImportOptions {
    /// Largest texture edge kept after decoding; `0` keeps source resolution.
    pub max_texture_dimension: u32,
    /// Tangent-space normal maps are only meaningful when the asset ships tangents.
    pub normal_maps: bool,
    pub metallic_roughness_maps: bool,
}

impl Default for GltfImportOptions {
    fn default() -> Self {
        Self {
            max_texture_dimension: 0,
            normal_maps: true,
            metallic_roughness_maps: true,
        }
    }
}

/// One glTF node that carries geometry. Mesh vertices stay in node-local space;
/// `transform` places the node in the scene, including all of its ancestors.
#[derive(Clone, Debug)]
pub struct GltfNode {
    pub name: String,
    pub transform: Mat4,
    pub meshes: Vec<Mesh>,
}

#[derive(Clone, Debug)]
pub struct GltfScene {
    pub materials: Vec<Material>,
    pub nodes: Vec<GltfNode>,
}

impl GltfScene {
    /// Flattens the hierarchy into world-space meshes.
    pub fn into_baked_meshes(self) -> (Vec<Material>, Vec<Mesh>) {
        let meshes = self
            .nodes
            .into_iter()
            .flat_map(|node| {
                node.meshes.into_iter().map(move |mut mesh| {
                    mesh.transform = node.transform;
                    mesh.bake_transform();
                    mesh
                })
            })
            .collect();
        (self.materials, meshes)
    }
}

/// Imports the default scene with world-space (baked) geometry.
pub fn import_gltf(path: impl AsRef<Path>) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError> {
    Ok(import_gltf_scene(path, GltfImportOptions::default())?.into_baked_meshes())
}

/// Imports the default scene, preserving node names and transforms.
pub fn import_gltf_scene(
    path: impl AsRef<Path>,
    options: GltfImportOptions,
) -> Result<GltfScene, GltfImportError> {
    let path = path.as_ref();
    let gltf::Gltf { document, blob } =
        gltf::Gltf::open(path).map_err(|error| GltfImportError::Gltf(error.to_string()))?;
    let buffers = gltf::import_buffers(&document, path.parent(), blob)
        .map_err(|error| GltfImportError::Gltf(error.to_string()))?;

    let mut textures = TextureLoader {
        base: path.parent().unwrap_or(Path::new(".")),
        buffers: &buffers,
        max_dimension: options.max_texture_dimension,
        cache: HashMap::new(),
    };
    let mut materials = document
        .materials()
        .map(|material| import_material(&material, &mut textures, options))
        .collect::<Result<Vec<_>, _>>()?;
    let default_material = materials.len();
    materials.push(default_material_for(default_material));

    let mut nodes = Vec::new();
    let roots: Vec<_> = match document
        .default_scene()
        .or_else(|| document.scenes().next())
    {
        Some(scene) => scene.nodes().collect(),
        None => document.nodes().collect(),
    };
    let mut stack: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|node| (node, Mat4::IDENTITY))
        .collect();
    while let Some((node, parent)) = stack.pop() {
        let transform = parent
            * Mat4 {
                m: node.transform().matrix(),
            };
        for child in node.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((child, transform));
        }
        let Some(source_mesh) = node.mesh() else {
            continue;
        };
        let name = node
            .name()
            .or(source_mesh.name())
            .unwrap_or("gltf_node")
            .to_string();
        let mut meshes = Vec::new();
        for (primitive_index, primitive) in source_mesh.primitives().enumerate() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let material_index = primitive.material().index().unwrap_or(default_material);
            let mesh_name = format!("{name}#{primitive_index}");
            let mut mesh = import_primitive(&primitive, &buffers, mesh_name)?;
            mesh.material_index = material_index;
            mesh.reflection_proxy = materials[material_index].specular_intensity > 0.5;
            meshes.push(mesh);
        }
        nodes.push(GltfNode {
            name,
            transform,
            meshes,
        });
    }

    Ok(GltfScene { materials, nodes })
}

fn import_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
    name: String,
) -> Result<Mesh, GltfImportError> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| GltfImportError::MissingPositions { mesh: name.clone() })?
        .collect();
    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(Iterator::collect)
        .unwrap_or_default();
    let texcoords: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|coords| coords.into_f32().collect())
        .unwrap_or_default();
    let tangents: Vec<[f32; 4]> = reader
        .read_tangents()
        .map(Iterator::collect)
        .unwrap_or_default();
    let colors: Vec<[f32; 4]> = reader
        .read_colors(0)
        .map(|colors| colors.into_rgba_f32().collect())
        .unwrap_or_default();

    let vertices = positions
        .iter()
        .enumerate()
        .map(|(index, &[x, y, z])| {
            let normal = normals.get(index).copied().unwrap_or([0.0, 1.0, 0.0]);
            let uv = texcoords.get(index).copied().unwrap_or([0.0, 0.0]);
            let tangent = tangents.get(index).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
            let color = colors.get(index).copied().unwrap_or([1.0; 4]);
            Vertex {
                position: Vec3::new(x, y, z),
                previous_position: Vec3::new(x, y, z),
                normal: Vec3::new(normal[0], normal[1], normal[2]),
                tangent: Vec3::new(tangent[0], tangent[1], tangent[2]),
                tangent_sign: tangent[3],
                uv: Vec2::new(uv[0], uv[1]),
                color: Color::new(color[0], color[1], color[2], color[3]),
            }
        })
        .collect::<Vec<_>>();

    let flat_indices: Vec<u32> = reader
        .read_indices()
        .map(|indices| indices.into_u32().collect())
        .unwrap_or_else(|| (0..positions.len() as u32).collect());
    if !flat_indices.len().is_multiple_of(3) {
        return Err(GltfImportError::InvalidIndexCount {
            mesh: name,
            count: flat_indices.len(),
        });
    }
    if let Some(&index) = flat_indices
        .iter()
        .find(|&&index| index as usize >= vertices.len())
    {
        return Err(GltfImportError::InvalidIndexValue {
            mesh: name,
            index,
            vertex_count: vertices.len(),
        });
    }

    Ok(Mesh {
        name,
        vertices,
        indices: flat_indices.as_chunks::<3>().0.to_vec(),
        material_index: 0,
        transform: Mat4::IDENTITY,
        shadow_class: ShadowCasterClass::Main,
        is_floor_or_ground: false,
        prepass_eligible: false,
        reflection_proxy: false,
        cloth: None,
    })
}

struct TextureLoader<'a> {
    base: &'a Path,
    buffers: &'a [gltf::buffer::Data],
    max_dimension: u32,
    cache: HashMap<usize, Arc<TextureImage>>,
}

impl TextureLoader<'_> {
    fn load(&mut self, texture: gltf::Texture<'_>) -> Result<Arc<TextureImage>, GltfImportError> {
        let image = texture.source();
        if let Some(cached) = self.cache.get(&image.index()) {
            return Ok(cached.clone());
        }
        let decoded = match image.source() {
            gltf::image::Source::Uri { uri, .. } => {
                let path = self.base.join(uri);
                TextureImage::from_path_with_max_dimension(&path, self.max_dimension).map_err(
                    |message| GltfImportError::Image(format!("{}: {message}", path.display())),
                )?
            }
            gltf::image::Source::View { view, .. } => {
                let buffer = &self.buffers[view.buffer().index()].0;
                let bytes = &buffer[view.offset()..view.offset() + view.length()];
                TextureImage::from_encoded_bytes_with_max_dimension(bytes, self.max_dimension)
                    .map_err(GltfImportError::Image)?
            }
        };
        let decoded = Arc::new(decoded);
        self.cache.insert(image.index(), decoded.clone());
        Ok(decoded)
    }
}

fn import_material(
    source: &gltf::Material<'_>,
    textures: &mut TextureLoader<'_>,
    options: GltfImportOptions,
) -> Result<Material, GltfImportError> {
    let pbr = source.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let metallic = pbr.metallic_factor();
    let roughness = pbr.roughness_factor();
    let base_color_texture = pbr
        .base_color_texture()
        .map(|info| textures.load(info.texture()))
        .transpose()?;
    let normal_texture = source
        .normal_texture()
        .filter(|_| options.normal_maps)
        .map(|info| textures.load(info.texture()))
        .transpose()?;
    let pbr_texture = pbr
        .metallic_roughness_texture()
        .filter(|_| options.metallic_roughness_maps)
        .map(|info| textures.load(info.texture()))
        .transpose()?;
    let texture_count = usize::from(base_color_texture.is_some())
        + usize::from(normal_texture.is_some())
        + usize::from(pbr_texture.is_some());
    let blended = source.alpha_mode() == gltf::material::AlphaMode::Blend;
    let class = if blended {
        MaterialClass::Glass
    } else if metallic > 0.5 {
        MaterialClass::Metal
    } else {
        MaterialClass::Default
    };
    let [er, eg, eb] = source.emissive_factor();
    let emission = Color::rgb(er, eg, eb);
    Ok(Material {
        name: source.name().unwrap_or("gltf_material").to_string(),
        class,
        base_color: Color::new(factor[0], factor[1], factor[2], factor[3]),
        base_color_texture,
        normal_texture,
        pbr_texture,
        roughness,
        metallic,
        specular_intensity: if blended { 0.8 } else { 0.25 + metallic * 0.65 },
        cavity: 1.0,
        material_id: source.index().unwrap_or(0).min(255) as u8,
        texture_sample_count: texture_count.clamp(1, 4),
        flags: MaterialFlags {
            receives_contact_shadows: true,
            uses_bent_normals: true,
            alpha_tested: source.alpha_mode() == gltf::material::AlphaMode::Mask,
            glass: blended,
            ..MaterialFlags::default()
        },
        emission,
    })
}

fn default_material_for(index: usize) -> Material {
    Material {
        name: "gltf_default".to_string(),
        class: MaterialClass::Default,
        base_color: Color::rgb(0.7, 0.7, 0.7),
        base_color_texture: None,
        normal_texture: None,
        pbr_texture: None,
        roughness: 0.7,
        metallic: 0.0,
        specular_intensity: 0.25,
        cavity: 1.0,
        material_id: index.min(255) as u8,
        texture_sample_count: 1,
        flags: MaterialFlags {
            receives_contact_shadows: true,
            uses_bent_normals: true,
            ..MaterialFlags::default()
        },
        emission: Color::BLACK,
    }
}
