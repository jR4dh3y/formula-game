//! Drivable surfaces and walls, loaded from the `collision.bin` written by
//! `tools/track/collision.py`.

use glam::{DVec2, DVec3};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    Asphalt,
    Kerb,
    Grass,
    Gravel,
    Concrete,
}

impl Surface {
    fn from_id(id: u8) -> Result<Self, String> {
        Ok(match id {
            0 => Self::Asphalt,
            1 => Self::Kerb,
            2 => Self::Grass,
            3 => Self::Gravel,
            4 => Self::Concrete,
            other => return Err(format!("unknown surface id {other}")),
        })
    }

    /// Friction relative to clean asphalt.
    pub fn grip(self) -> f64 {
        match self {
            Self::Asphalt => 1.0,
            Self::Kerb => 0.93,
            Self::Concrete => 0.95,
            Self::Grass => 0.55,
            Self::Gravel => 0.5,
        }
    }

    /// Extra rolling drag coefficient (N per N of load) from soft surfaces.
    pub fn rolling_drag(self) -> f64 {
        match self {
            Self::Asphalt | Self::Concrete => 0.012,
            Self::Kerb => 0.02,
            Self::Grass => 0.12,
            Self::Gravel => 0.45,
        }
    }

    /// Peak-to-peak vertical shake (metres) felt over this surface.
    pub fn roughness(self) -> f64 {
        match self {
            Self::Asphalt | Self::Concrete => 0.0,
            Self::Kerb => 0.012,
            Self::Grass => 0.008,
            Self::Gravel => 0.02,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GroundHit {
    pub height: f64,
    pub normal: DVec3,
    pub surface: Surface,
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    a: DVec3,
    b: DVec3,
    c: DVec3,
    surface: Surface,
}

#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub a: DVec2,
    pub b: DVec2,
}

/// Uniform XZ grid of triangles and wall segments.
pub struct CollisionWorld {
    triangles: Vec<Triangle>,
    walls: Vec<Wall>,
    cell_size: f64,
    triangle_cells: HashMap<(i32, i32), Vec<u32>>,
    wall_cells: HashMap<(i32, i32), Vec<u32>>,
}

const MAGIC: &[u8; 4] = b"F1CL";

impl CollisionWorld {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader { bytes, offset: 0 };
        if reader.take(4)? != MAGIC {
            return Err("collision data has the wrong magic".to_string());
        }
        let version = reader.u32()?;
        if version != 1 {
            return Err(format!("unsupported collision version {version}"));
        }
        let triangle_count = reader.u32()? as usize;
        let wall_count = reader.u32()? as usize;
        let mut corners = Vec::with_capacity(triangle_count * 3);
        for _ in 0..triangle_count * 3 {
            corners.push(DVec3::new(reader.f32()?, reader.f32()?, reader.f32()?));
        }
        let surface_ids = reader.take(triangle_count)?.to_vec();
        reader.offset = reader.offset.next_multiple_of(4);
        let mut walls = Vec::with_capacity(wall_count);
        for _ in 0..wall_count {
            walls.push(Wall {
                a: DVec2::new(reader.f32()?, reader.f32()?),
                b: DVec2::new(reader.f32()?, reader.f32()?),
            });
        }
        let triangles = corners
            .chunks_exact(3)
            .zip(surface_ids)
            .map(|(corner, id)| {
                Ok(Triangle {
                    a: corner[0],
                    b: corner[1],
                    c: corner[2],
                    surface: Surface::from_id(id)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self::new(triangles, walls))
    }

    fn new(triangles: Vec<Triangle>, walls: Vec<Wall>) -> Self {
        let cell_size = 16.0;
        let mut triangle_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (index, triangle) in triangles.iter().enumerate() {
            let min = triangle.a.min(triangle.b).min(triangle.c);
            let max = triangle.a.max(triangle.b).max(triangle.c);
            for cell in cells_between(min.x, min.z, max.x, max.z, cell_size) {
                triangle_cells.entry(cell).or_default().push(index as u32);
            }
        }
        let mut wall_cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (index, wall) in walls.iter().enumerate() {
            let min = wall.a.min(wall.b);
            let max = wall.a.max(wall.b);
            for cell in cells_between(min.x, min.y, max.x, max.y, cell_size) {
                wall_cells.entry(cell).or_default().push(index as u32);
            }
        }
        Self {
            triangles,
            walls,
            cell_size,
            triangle_cells,
            wall_cells,
        }
    }

    /// Highest drivable surface under `point` that is not far above it.
    pub fn ground(&self, point: DVec3) -> Option<GroundHit> {
        let cell = cell_of(point.x, point.z, self.cell_size);
        let candidates = self.triangle_cells.get(&cell)?;
        let mut best: Option<GroundHit> = None;
        for &index in candidates {
            let triangle = &self.triangles[index as usize];
            let Some(height) = height_on_triangle(triangle, point.x, point.z) else {
                continue;
            };
            // Ignore bridges and overhangs well above the probe.
            if height > point.y + 2.5 {
                continue;
            }
            if best.is_none_or(|hit| height > hit.height) {
                let normal = (triangle.b - triangle.a)
                    .cross(triangle.c - triangle.a)
                    .normalize_or_zero();
                let normal = if normal.y < 0.0 { -normal } else { normal };
                best = Some(GroundHit {
                    height,
                    normal,
                    surface: triangle.surface,
                });
            }
        }
        best
    }

    /// Wall segments whose cells overlap the circle.
    pub fn walls_near(&self, center: DVec2, radius: f64) -> impl Iterator<Item = &Wall> {
        let mut indices: Vec<u32> = cells_between(
            center.x - radius,
            center.y - radius,
            center.x + radius,
            center.y + radius,
            self.cell_size,
        )
        .filter_map(|cell| self.wall_cells.get(&cell))
        .flatten()
        .copied()
        .collect();
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(move |index| &self.walls[index as usize])
    }

    pub fn wall_count(&self) -> usize {
        self.walls.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }
}

fn cell_of(x: f64, z: f64, size: f64) -> (i32, i32) {
    ((x / size).floor() as i32, (z / size).floor() as i32)
}

fn cells_between(
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
    size: f64,
) -> impl Iterator<Item = (i32, i32)> {
    let (x0, z0) = cell_of(min_x, min_z, size);
    let (x1, z1) = cell_of(max_x, max_z, size);
    (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
}

fn height_on_triangle(triangle: &Triangle, x: f64, z: f64) -> Option<f64> {
    let (a, b, c) = (triangle.a, triangle.b, triangle.c);
    let denominator = (b.z - c.z) * (a.x - c.x) + (c.x - b.x) * (a.z - c.z);
    if denominator.abs() < 1e-12 {
        return None;
    }
    let u = ((b.z - c.z) * (x - c.x) + (c.x - b.x) * (z - c.z)) / denominator;
    let v = ((c.z - a.z) * (x - c.x) + (a.x - c.x) * (z - c.z)) / denominator;
    let w = 1.0 - u - v;
    const EPSILON: f64 = -1e-6;
    (u >= EPSILON && v >= EPSILON && w >= EPSILON).then(|| u * a.y + v * b.y + w * c.y)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self.offset + count;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or("collision data is truncated")?;
        self.offset = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn f32(&mut self) -> Result<f64, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")) as f64)
    }
}

#[cfg(test)]
pub(crate) fn flat_world(half_size: f64, surface: Surface, walls: Vec<Wall>) -> CollisionWorld {
    let corners = [
        DVec3::new(-half_size, 0.0, -half_size),
        DVec3::new(half_size, 0.0, -half_size),
        DVec3::new(half_size, 0.0, half_size),
        DVec3::new(-half_size, 0.0, half_size),
    ];
    CollisionWorld::new(
        vec![
            Triangle {
                a: corners[0],
                b: corners[1],
                c: corners[2],
                surface,
            },
            Triangle {
                a: corners[0],
                b: corners[2],
                c: corners[3],
                surface,
            },
        ],
        walls,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_interpolates_triangle_height() {
        let world = CollisionWorld::new(
            vec![Triangle {
                a: DVec3::new(0.0, 0.0, 0.0),
                b: DVec3::new(10.0, 10.0, 0.0),
                c: DVec3::new(0.0, 0.0, 10.0),
                surface: Surface::Kerb,
            }],
            Vec::new(),
        );
        let hit = world.ground(DVec3::new(5.0, 20.0, 1.0)).expect("hit");
        assert!((hit.height - 5.0).abs() < 1e-9);
        assert_eq!(hit.surface, Surface::Kerb);
        assert!(hit.normal.y > 0.0);
        assert!(world.ground(DVec3::new(9.0, 20.0, 9.0)).is_none());
    }

    #[test]
    fn binary_round_trip() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        for value in [1_u32, 1, 1] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in [0.0_f32, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[3, 0, 0, 0]);
        for value in [1.0_f32, 2.0, 3.0, 4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let world = CollisionWorld::from_bytes(&bytes).expect("parse");
        assert_eq!(world.triangle_count(), 1);
        assert_eq!(world.wall_count(), 1);
        assert_eq!(
            world.ground(DVec3::new(1.0, 1.0, 1.0)).unwrap().surface,
            Surface::Gravel
        );
        assert_eq!(world.walls_near(DVec2::new(2.0, 3.0), 1.0).count(), 1);
    }
}
