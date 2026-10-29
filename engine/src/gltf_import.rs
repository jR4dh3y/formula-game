use crate::asset::ShadowCasterClass;
use crate::math::{Color, Mat4, Vec2, Vec3};
use crate::scene::{Material, MaterialClass, MaterialFlags, Mesh, TextureImage, Vertex};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::{error::Error, fmt};

#[derive(Clone, Debug, PartialEq)]
pub enum GltfImportError {
    Gltf(String),
    UnsupportedImageFormat(String),
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
            Self::UnsupportedImageFormat(format) => {
                write!(formatter, "unsupported glTF image format: {format}")
            }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfImportBudget {
    pub max_meshes: usize,
    pub max_vertices: usize,
    pub max_triangles: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfPrimitiveInfo<'a> {
    pub mesh_name: &'a str,
    pub material_name: Option<&'a str>,
    pub vertex_count: usize,
    pub triangle_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GltfTextureLimits {
    pub base_color: Option<u32>,
    pub normal: Option<u32>,
    pub pbr: Option<u32>,
}

impl GltfTextureLimits {
    pub const fn base_color(max_dimension: u32) -> Self {
        Self {
            base_color: Some(max_dimension),
            normal: None,
            pbr: None,
        }
    }
}

pub fn import_gltf(path: impl AsRef<Path>) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError> {
    let (document, buffers, images) =
        gltf::import(path).map_err(|error| GltfImportError::Gltf(error.to_string()))?;

    let mut materials: Vec<Material> = document
        .materials()
        .map(|material| import_material(&material, &images))
        .collect::<Result<_, _>>()?;
    if materials.is_empty() {
        materials.push(default_material(0));
    }

    let mut meshes = Vec::new();
    for node in document.nodes() {
        let Some(source_mesh) = node.mesh() else {
            continue;
        };
        let transform = Mat4 {
            m: node.transform().matrix(),
        };
        for (primitive_index, primitive) in source_mesh.primitives().enumerate() {
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let positions: Vec<_> = reader
                .read_positions()
                .ok_or_else(|| GltfImportError::MissingPositions {
                    mesh: source_mesh.name().unwrap_or("unnamed").to_string(),
                })?
                .collect();
            let normals: Vec<_> = reader
                .read_normals()
                .map(Iterator::collect)
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            let texcoords: Vec<_> = reader
                .read_tex_coords(0)
                .map(|coords| coords.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
            let tangents: Vec<_> = reader
                .read_tangents()
                .map(Iterator::collect)
                .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 1.0]; positions.len()]);
            let colors: Vec<_> = reader
                .read_colors(0)
                .map(|colors| colors.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.0, 1.0, 1.0, 1.0]; positions.len()]);

            let vertices = positions
                .iter()
                .enumerate()
                .map(|(index, position)| {
                    let normal = normals.get(index).copied().unwrap_or([0.0, 1.0, 0.0]);
                    let uv = texcoords.get(index).copied().unwrap_or([0.0, 0.0]);
                    let tangent = tangents.get(index).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
                    let color = colors.get(index).copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
                    Vertex {
                        position: Vec3::new(position[0], position[1], position[2]),
                        previous_position: Vec3::new(position[0], position[1], position[2]),
                        normal: Vec3::new(normal[0], normal[1], normal[2]),
                        tangent: Vec3::new(tangent[0], tangent[1], tangent[2]),
                        tangent_sign: tangent[3],
                        uv: Vec2::new(uv[0], uv[1]),
                        color: Color::new(color[0], color[1], color[2], color[3]),
                    }
                })
                .collect();

            let flat_indices: Vec<u32> = reader
                .read_indices()
                .map(|indices| indices.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            if !flat_indices.len().is_multiple_of(3) {
                return Err(GltfImportError::InvalidIndexCount {
                    mesh: source_mesh.name().unwrap_or("unnamed").to_string(),
                    count: flat_indices.len(),
                });
            }
            let indices = flat_indices.as_chunks::<3>().0.to_vec();
            let material_index = primitive.material().index().unwrap_or(0);
            let reflective = materials
                .get(material_index)
                .is_some_and(|material| material.specular_intensity > 0.5);
            meshes.push(Mesh {
                name: format!(
                    "{}_{primitive_index}",
                    source_mesh.name().unwrap_or("gltf_mesh")
                ),
                vertices,
                indices,
                material_index,
                transform,
                shadow_class: ShadowCasterClass::Main,
                is_floor_or_ground: false,
                prepass_eligible: false,
                reflection_proxy: reflective,
                cloth: None,
            });
        }
    }
    Ok((materials, meshes))
}

pub fn import_gltf_with_max_texture_dimension(
    path: impl AsRef<Path>,
    max_texture_dimension: u32,
) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError> {
    import_gltf_with_max_texture_dimension_filter(path, max_texture_dimension, |_, _| true)
}

pub fn import_gltf_with_max_texture_dimension_filter<F>(
    path: impl AsRef<Path>,
    max_texture_dimension: u32,
    keep_primitive: F,
) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError>
where
    F: Fn(&str, Option<&str>) -> bool,
{
    import_gltf_with_max_texture_dimension_filter_budget(
        path,
        max_texture_dimension,
        keep_primitive,
        None,
    )
}

pub fn import_gltf_with_max_texture_dimension_filter_budget<F>(
    path: impl AsRef<Path>,
    max_texture_dimension: u32,
    keep_primitive: F,
    budget: Option<GltfImportBudget>,
) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError>
where
    F: Fn(&str, Option<&str>) -> bool,
{
    import_gltf_with_max_texture_dimension_filter_budget_by_primitive(
        path,
        max_texture_dimension,
        |info| keep_primitive(info.mesh_name, info.material_name),
        budget,
    )
}

pub fn import_gltf_with_max_texture_dimension_filter_budget_by_primitive<F>(
    path: impl AsRef<Path>,
    max_texture_dimension: u32,
    keep_primitive: F,
    budget: Option<GltfImportBudget>,
) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError>
where
    for<'a> F: Fn(GltfPrimitiveInfo<'a>) -> bool,
{
    import_gltf_with_texture_limits_filter_budget_by_primitive(path, keep_primitive, budget, |_| {
        GltfTextureLimits::base_color(max_texture_dimension)
    })
}

pub fn import_gltf_with_texture_limits_filter_budget_by_primitive<F, P>(
    path: impl AsRef<Path>,
    keep_primitive: F,
    budget: Option<GltfImportBudget>,
    texture_limits: P,
) -> Result<(Vec<Material>, Vec<Mesh>), GltfImportError>
where
    for<'a> F: Fn(GltfPrimitiveInfo<'a>) -> bool,
    P: Fn(Option<&str>) -> GltfTextureLimits,
{
    let path = path.as_ref();
    let base = path.parent().unwrap_or_else(|| Path::new("./"));
    let file = fs::File::open(path).map_err(|error| GltfImportError::Gltf(error.to_string()))?;
    let reader = BufReader::new(file);
    let gltf = gltf::Gltf::from_reader_without_validation(reader)
        .map_err(|error| GltfImportError::Gltf(error.to_string()))?;
    let gltf::Gltf { document, blob } = gltf;
    let buffers = gltf::import_buffers(&document, Some(base), blob)
        .map_err(|error| GltfImportError::Gltf(error.to_string()))?;

    let mut meshes = Vec::new();
    let mut imported_vertices = 0usize;
    let mut imported_triangles = 0usize;
    'nodes: for node in document.nodes() {
        let Some(source_mesh) = node.mesh() else {
            continue;
        };
        let mesh_name = source_mesh.name().unwrap_or("gltf_mesh");
        let transform = Mat4 {
            m: node.transform().matrix(),
        };
        for (primitive_index, primitive) in source_mesh.primitives().enumerate() {
            let primitive_material = primitive.material();
            let primitive_info = GltfPrimitiveInfo {
                mesh_name,
                material_name: primitive_material.name(),
                vertex_count: gltf_position_count(&primitive),
                triangle_count: gltf_triangle_count(&primitive),
            };
            if !keep_primitive(primitive_info) {
                continue;
            }
            if budget.is_some_and(|budget| {
                meshes.len() >= budget.max_meshes
                    || imported_vertices >= budget.max_vertices
                    || imported_triangles >= budget.max_triangles
            }) {
                break 'nodes;
            }
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()].0));
            let positions: Vec<_> = reader
                .read_positions()
                .ok_or_else(|| GltfImportError::MissingPositions {
                    mesh: mesh_name.to_string(),
                })?
                .collect();
            let flat_indices: Vec<u32> = reader
                .read_indices()
                .map(|indices| indices.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());
            if !flat_indices.len().is_multiple_of(3) {
                return Err(GltfImportError::InvalidIndexCount {
                    mesh: mesh_name.to_string(),
                    count: flat_indices.len(),
                });
            }
            let compact_vertex_count =
                referenced_vertex_count(mesh_name, positions.len(), &flat_indices)?;
            let triangle_count = flat_indices.len() / 3;
            if budget.is_some_and(|budget| {
                meshes.len() + 1 > budget.max_meshes
                    || imported_vertices + compact_vertex_count > budget.max_vertices
                    || imported_triangles + triangle_count > budget.max_triangles
            }) {
                break 'nodes;
            }
            let normals: Vec<_> = reader
                .read_normals()
                .map(Iterator::collect)
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            let texcoords: Vec<_> = reader
                .read_tex_coords(0)
                .map(|coords| coords.into_f32().collect())
                .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
            let tangents: Vec<_> = reader
                .read_tangents()
                .map(Iterator::collect)
                .unwrap_or_else(|| vec![[1.0, 0.0, 0.0, 1.0]; positions.len()]);
            let colors: Vec<_> = reader
                .read_colors(0)
                .map(|colors| colors.into_rgba_f32().collect())
                .unwrap_or_else(|| vec![[1.0, 1.0, 1.0, 1.0]; positions.len()]);

            let (vertices, indices) = compact_indexed_geometry(
                mesh_name,
                &positions,
                &normals,
                &texcoords,
                &tangents,
                &colors,
                &flat_indices,
            )?;
            let material_index = primitive.material().index().unwrap_or(0);
            imported_vertices += vertices.len();
            imported_triangles += indices.len();
            meshes.push(Mesh {
                name: format!("{mesh_name}_{primitive_index}"),
                vertices,
                indices,
                material_index,
                transform,
                shadow_class: ShadowCasterClass::Main,
                is_floor_or_ground: false,
                prepass_eligible: false,
                reflection_proxy: false,
                cloth: None,
            });
        }
    }

    let used_materials: HashSet<_> = meshes.iter().map(|mesh| mesh.material_index).collect();
    let mut image_cache = HashMap::new();
    let mut materials: Vec<Material> = document
        .materials()
        .map(|material| match material.index() {
            Some(index) if used_materials.contains(&index) => {
                let limits = texture_limits(material.name());
                import_material_limited(&material, base, &buffers, &mut image_cache, limits)
            }
            _ => Ok(skipped_material(&material)),
        })
        .collect::<Result<_, _>>()?;
    if materials.is_empty() {
        materials.push(default_material(0));
    }
    for mesh in &mut meshes {
        mesh.reflection_proxy = materials
            .get(mesh.material_index)
            .is_some_and(|material| material.specular_intensity > 0.5);
    }

    Ok((materials, meshes))
}

fn gltf_position_count(primitive: &gltf::Primitive<'_>) -> usize {
    primitive
        .attributes()
        .find_map(|(semantic, accessor)| match semantic {
            gltf::Semantic::Positions => Some(accessor.count()),
            _ => None,
        })
        .unwrap_or(0)
}

fn gltf_triangle_count(primitive: &gltf::Primitive<'_>) -> usize {
    let index_count = primitive
        .indices()
        .map(|accessor| accessor.count())
        .unwrap_or_else(|| gltf_position_count(primitive));
    index_count / 3
}

fn referenced_vertex_count(
    mesh_name: &str,
    vertex_count: usize,
    flat_indices: &[u32],
) -> Result<usize, GltfImportError> {
    let mut referenced = vec![false; vertex_count];
    let mut count = 0usize;
    for &source_index in flat_indices {
        let source_index_usize = source_index as usize;
        if source_index_usize >= vertex_count {
            return Err(GltfImportError::InvalidIndexValue {
                mesh: mesh_name.to_string(),
                index: source_index,
                vertex_count,
            });
        }
        if !referenced[source_index_usize] {
            referenced[source_index_usize] = true;
            count += 1;
        }
    }
    Ok(count)
}

fn compact_indexed_geometry(
    mesh_name: &str,
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    texcoords: &[[f32; 2]],
    tangents: &[[f32; 4]],
    colors: &[[f32; 4]],
    flat_indices: &[u32],
) -> Result<(Vec<Vertex>, Vec<[u32; 3]>), GltfImportError> {
    let mut remap = vec![u32::MAX; positions.len()];
    let mut vertices = Vec::new();
    let mut compact_indices = Vec::with_capacity(flat_indices.len());
    for &source_index in flat_indices {
        let source_index_usize = source_index as usize;
        if source_index_usize >= positions.len() {
            return Err(GltfImportError::InvalidIndexValue {
                mesh: mesh_name.to_string(),
                index: source_index,
                vertex_count: positions.len(),
            });
        }
        let mut compact_index = remap[source_index_usize];
        if compact_index == u32::MAX {
            let position = positions[source_index_usize];
            let normal = normals
                .get(source_index_usize)
                .copied()
                .unwrap_or([0.0, 1.0, 0.0]);
            let uv = texcoords
                .get(source_index_usize)
                .copied()
                .unwrap_or([0.0, 0.0]);
            let tangent = tangents
                .get(source_index_usize)
                .copied()
                .unwrap_or([1.0, 0.0, 0.0, 1.0]);
            let color = colors
                .get(source_index_usize)
                .copied()
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            compact_index = vertices.len() as u32;
            remap[source_index_usize] = compact_index;
            vertices.push(Vertex {
                position: Vec3::new(position[0], position[1], position[2]),
                previous_position: Vec3::new(position[0], position[1], position[2]),
                normal: Vec3::new(normal[0], normal[1], normal[2]),
                tangent: Vec3::new(tangent[0], tangent[1], tangent[2]),
                tangent_sign: tangent[3],
                uv: Vec2::new(uv[0], uv[1]),
                color: Color::new(color[0], color[1], color[2], color[3]),
            });
        }
        compact_indices.push(compact_index);
    }
    let indices = compact_indices.as_chunks::<3>().0.to_vec();
    Ok((vertices, indices))
}

fn import_material(
    source: &gltf::Material<'_>,
    images: &[gltf::image::Data],
) -> Result<Material, GltfImportError> {
    let pbr = source.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let metallic = pbr.metallic_factor();
    let roughness = pbr.roughness_factor();
    let texture = pbr
        .base_color_texture()
        .map(|texture| texture_from_gltf(&images[texture.texture().source().index()]))
        .transpose()?;
    let normal_texture = source
        .normal_texture()
        .map(|texture| texture_from_gltf(&images[texture.texture().source().index()]))
        .transpose()?;
    let pbr_texture = pbr
        .metallic_roughness_texture()
        .map(|texture| texture_from_gltf(&images[texture.texture().source().index()]))
        .transpose()?;
    let texture_count = usize::from(texture.is_some())
        + usize::from(pbr.metallic_roughness_texture().is_some())
        + usize::from(source.normal_texture().is_some())
        + usize::from(source.occlusion_texture().is_some());
    let class = if metallic > 0.5 {
        MaterialClass::Metal
    } else {
        MaterialClass::Default
    };
    Ok(Material {
        name: source.name().unwrap_or("gltf_material").to_string(),
        class,
        base_color: Color::new(factor[0], factor[1], factor[2], factor[3]),
        base_color_texture: texture,
        normal_texture,
        pbr_texture,
        roughness,
        metallic,
        specular_intensity: 0.25 + metallic * 0.65,
        cavity: 1.0,
        material_id: source.index().unwrap_or(0).min(255) as u8,
        texture_sample_count: texture_count.clamp(1, 4),
        flags: MaterialFlags {
            receives_contact_shadows: true,
            uses_bent_normals: true,
            alpha_tested: source.alpha_mode() == gltf::material::AlphaMode::Mask,
            ..MaterialFlags::default()
        },
        emission: Color::BLACK,
    })
}

fn import_material_limited(
    source: &gltf::Material<'_>,
    base: &Path,
    buffers: &[gltf::buffer::Data],
    image_cache: &mut HashMap<(usize, u32), TextureImage>,
    texture_limits: GltfTextureLimits,
) -> Result<Material, GltfImportError> {
    let pbr = source.pbr_metallic_roughness();
    let factor = pbr.base_color_factor();
    let metallic = pbr.metallic_factor();
    let roughness = pbr.roughness_factor();
    let texture = pbr
        .base_color_texture()
        .zip(texture_limits.base_color)
        .map(|(texture, max_dimension)| {
            texture_from_gltf_source(
                texture.texture().source(),
                base,
                buffers,
                image_cache,
                max_dimension,
            )
        })
        .transpose()?;
    let normal_texture = source
        .normal_texture()
        .zip(texture_limits.normal)
        .map(|(texture, max_dimension)| {
            texture_from_gltf_source(
                texture.texture().source(),
                base,
                buffers,
                image_cache,
                max_dimension,
            )
        })
        .transpose()?;
    let pbr_texture = pbr
        .metallic_roughness_texture()
        .zip(texture_limits.pbr)
        .map(|(texture, max_dimension)| {
            texture_from_gltf_source(
                texture.texture().source(),
                base,
                buffers,
                image_cache,
                max_dimension,
            )
        })
        .transpose()?;
    let texture_count = usize::from(texture.is_some())
        + usize::from(normal_texture.is_some())
        + usize::from(pbr_texture.is_some())
        + usize::from(source.occlusion_texture().is_some() && texture_limits.pbr.is_some());
    let class = if metallic > 0.5 {
        MaterialClass::Metal
    } else {
        MaterialClass::Default
    };
    Ok(Material {
        name: source.name().unwrap_or("gltf_material").to_string(),
        class,
        base_color: Color::new(factor[0], factor[1], factor[2], factor[3]),
        base_color_texture: texture,
        normal_texture,
        pbr_texture,
        roughness,
        metallic,
        specular_intensity: 0.25 + metallic * 0.65,
        cavity: 1.0,
        material_id: source.index().unwrap_or(0).min(255) as u8,
        texture_sample_count: texture_count.clamp(1, 4),
        flags: MaterialFlags {
            receives_contact_shadows: true,
            uses_bent_normals: true,
            alpha_tested: source.alpha_mode() == gltf::material::AlphaMode::Mask,
            ..MaterialFlags::default()
        },
        emission: Color::BLACK,
    })
}

fn default_material(index: usize) -> Material {
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

fn skipped_material(source: &gltf::Material<'_>) -> Material {
    let mut material = default_material(source.index().unwrap_or(0));
    material.name = source.name().unwrap_or("gltf_material").to_string();
    material
}

fn texture_from_gltf(image: &gltf::image::Data) -> Result<TextureImage, GltfImportError> {
    use gltf::image::Format;
    let pixels = match image.format {
        Format::R8G8B8 => image
            .pixels
            .as_chunks::<3>()
            .0
            .iter()
            .map(|pixel| Color::rgb(byte(pixel[0]), byte(pixel[1]), byte(pixel[2])))
            .collect(),
        Format::R8G8B8A8 => image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                Color::new(
                    byte(pixel[0]),
                    byte(pixel[1]),
                    byte(pixel[2]),
                    byte(pixel[3]),
                )
            })
            .collect(),
        other => {
            return Err(GltfImportError::UnsupportedImageFormat(format!(
                "{other:?}"
            )))
        }
    };
    Ok(TextureImage {
        width: image.width,
        height: image.height,
        pixels,
    })
}

fn texture_from_gltf_source(
    image: gltf::image::Image<'_>,
    base: &Path,
    buffers: &[gltf::buffer::Data],
    image_cache: &mut HashMap<(usize, u32), TextureImage>,
    max_texture_dimension: u32,
) -> Result<TextureImage, GltfImportError> {
    let cache_key = (image.index(), max_texture_dimension);
    if let Some(texture) = image_cache.get(&cache_key) {
        return Ok(texture.clone());
    }

    let texture = match image.source() {
        gltf::image::Source::Uri { uri, .. } => {
            let path = base.join(uri);
            TextureImage::from_path_with_max_dimension(&path, max_texture_dimension).map_err(
                |message| GltfImportError::Gltf(format!("{}: {message}", path.display())),
            )?
        }
        gltf::image::Source::View { view, .. } => {
            let parent_buffer = &buffers[view.buffer().index()].0;
            let begin = view.offset();
            let end = begin + view.length();
            TextureImage::from_encoded_bytes_with_max_dimension(
                &parent_buffer[begin..end],
                max_texture_dimension,
            )
            .map_err(GltfImportError::Gltf)?
        }
    };
    image_cache.insert(cache_key, texture.clone());
    Ok(texture)
}

fn byte(value: u8) -> f32 {
    value as f32 / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_indexed_geometry_drops_unreferenced_vertices() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
        ];
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        let texcoords = vec![[0.0, 0.0]; positions.len()];
        let tangents = vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
        let colors = vec![[1.0, 1.0, 1.0, 1.0]; positions.len()];
        let flat_indices = [4, 2, 4, 2, 1, 4];

        let compact_count =
            referenced_vertex_count("mesh", positions.len(), &flat_indices).unwrap();
        let (vertices, indices) = compact_indexed_geometry(
            "mesh",
            &positions,
            &normals,
            &texcoords,
            &tangents,
            &colors,
            &flat_indices,
        )
        .unwrap();

        assert_eq!(compact_count, 3);
        assert_eq!(vertices.len(), 3);
        assert_eq!(vertices[0].position, Vec3::new(4.0, 0.0, 0.0));
        assert_eq!(vertices[1].position, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(vertices[2].position, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(indices, vec![[0, 1, 0], [1, 2, 0]]);
    }
}
