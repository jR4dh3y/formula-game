use crate::math::{Vec2, Vec3};
use crate::scene::Vertex;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const EPSILON: f32 = 1.0e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothSettings {
    pub gravity: Vec3,
    pub wind: Vec3,
    pub damping: f32,
    pub constraint_iterations: u32,
    pub structural_stiffness: f32,
    pub bending_stiffness: f32,
    pub particle_radius: f32,
    pub collision_friction: f32,
}

impl Default for ClothSettings {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            wind: Vec3::ZERO,
            damping: 0.995,
            constraint_iterations: 8,
            structural_stiffness: 1.0,
            bending_stiffness: 0.5,
            particle_radius: 0.005,
            collision_friction: 0.2,
        }
    }
}

impl ClothSettings {
    fn validate(self) -> Result<(), ClothError> {
        if !finite(self.gravity) {
            return Err(ClothError::InvalidSettings("gravity", self.gravity.x));
        }
        if !finite(self.wind) {
            return Err(ClothError::InvalidSettings("wind", self.wind.x));
        }
        for (field, value, valid) in [
            ("damping", self.damping, (0.0..=1.0).contains(&self.damping)),
            (
                "structural_stiffness",
                self.structural_stiffness,
                (0.0..=1.0).contains(&self.structural_stiffness),
            ),
            (
                "bending_stiffness",
                self.bending_stiffness,
                (0.0..=1.0).contains(&self.bending_stiffness),
            ),
            (
                "particle_radius",
                self.particle_radius,
                self.particle_radius >= 0.0,
            ),
            (
                "collision_friction",
                self.collision_friction,
                (0.0..=1.0).contains(&self.collision_friction),
            ),
        ] {
            if !value.is_finite() || !valid {
                return Err(ClothError::InvalidSettings(field, value));
            }
        }
        if self.constraint_iterations == 0 {
            return Err(ClothError::InvalidSettings("constraint_iterations", 0.0));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothPin {
    pub vertex: usize,
    pub position: Option<Vec3>,
}

impl ClothPin {
    pub const fn at_initial(vertex: usize) -> Self {
        Self {
            vertex,
            position: None,
        }
    }

    pub const fn at(vertex: usize, position: Vec3) -> Self {
        Self {
            vertex,
            position: Some(position),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothDefinition {
    pub settings: ClothSettings,
    pub pins: Vec<ClothPin>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClothColliderSnapshot {
    Floor { height: f32 },
    Sphere { center: Vec3, radius: f32 },
    Aabb { min: Vec3, max: Vec3 },
}

impl ClothColliderSnapshot {
    pub const fn floor(height: f32) -> Self {
        Self::Floor { height }
    }

    pub const fn sphere(center: Vec3, radius: f32) -> Self {
        Self::Sphere { center, radius }
    }

    pub const fn aabb(min: Vec3, max: Vec3) -> Self {
        Self::Aabb { min, max }
    }

    fn validate(self, index: usize) -> Result<(), ClothError> {
        let invalid = match self {
            Self::Floor { height } => !height.is_finite(),
            Self::Sphere { center, radius } => {
                !finite(center) || !radius.is_finite() || radius < 0.0
            }
            Self::Aabb { min, max } => !finite(min) || !finite(max),
        };
        invalid
            .then_some(ClothError::InvalidCollider(index))
            .map_or(Ok(()), Err)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClothError {
    EmptyVertexBuffer,
    NonFiniteVertex(usize),
    InvalidTriangleIndex(usize, usize, u32, usize),
    InvalidPin(usize, usize),
    NonFinitePin(usize),
    InvalidSettings(&'static str, f32),
    InvalidDeltaTime(f32),
    InvalidCollider(usize),
    VertexBufferLength(usize, usize),
}

impl fmt::Display for ClothError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyVertexBuffer => formatter.write_str("cloth has no vertices"),
            Self::NonFiniteVertex(vertex) => {
                write!(formatter, "cloth vertex {vertex} has a non-finite position")
            }
            Self::InvalidTriangleIndex(triangle, corner, vertex, vertex_count) => write!(
                formatter,
                "cloth triangle {triangle} corner {corner} references vertex {vertex}, but the mesh has {vertex_count} vertices"
            ),
            Self::InvalidPin(vertex, vertex_count) => write!(
                formatter,
                "cloth pin references vertex {vertex}, but the mesh has {vertex_count} vertices"
            ),
            Self::NonFinitePin(vertex) => {
                write!(formatter, "cloth pin {vertex} has a non-finite position")
            }
            Self::InvalidSettings(field, value) => {
                write!(formatter, "invalid cloth setting {field}={value}")
            }
            Self::InvalidDeltaTime(value) => write!(formatter, "invalid cloth delta time {value}"),
            Self::InvalidCollider(index) => write!(formatter, "invalid cloth collider {index}"),
            Self::VertexBufferLength(expected, actual) => write!(
                formatter,
                "cloth expected {expected} output vertices, but received {actual}"
            ),
        }
    }
}

impl std::error::Error for ClothError {}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DistanceConstraint {
    a: usize,
    b: usize,
    rest_length: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClothState {
    positions: Vec<Vec3>,
    previous_positions: Vec<Vec3>,
    inverse_masses: Vec<f32>,
    anchors: Vec<Option<Vec3>>,
    triangles: Vec<[usize; 3]>,
    structural: Vec<DistanceConstraint>,
    bending: Vec<DistanceConstraint>,
    uvs: Vec<Vec2>,
    settings: ClothSettings,
}

impl ClothState {
    pub fn new(
        vertices: &[Vertex],
        triangle_indices: &[[u32; 3]],
        definition: &ClothDefinition,
    ) -> Result<Self, ClothError> {
        definition.settings.validate()?;
        if vertices.is_empty() {
            return Err(ClothError::EmptyVertexBuffer);
        }
        for (index, vertex) in vertices.iter().enumerate() {
            if !finite(vertex.position) || !finite(vertex.previous_position) {
                return Err(ClothError::NonFiniteVertex(index));
            }
        }

        let mut triangles = Vec::with_capacity(triangle_indices.len());
        for (triangle, source) in triangle_indices.iter().enumerate() {
            let mut converted = [0usize; 3];
            for (corner, &index) in source.iter().enumerate() {
                let index = index as usize;
                if index >= vertices.len() {
                    return Err(ClothError::InvalidTriangleIndex(
                        triangle,
                        corner,
                        source[corner],
                        vertices.len(),
                    ));
                }
                converted[corner] = index;
            }
            triangles.push(converted);
        }

        let mut anchors = vec![None; vertices.len()];
        for pin in &definition.pins {
            if pin.vertex >= vertices.len() {
                return Err(ClothError::InvalidPin(pin.vertex, vertices.len()));
            }
            let anchor = pin.position.unwrap_or(vertices[pin.vertex].position);
            if !finite(anchor) {
                return Err(ClothError::NonFinitePin(pin.vertex));
            }
            anchors[pin.vertex] = Some(anchor);
        }

        let positions: Vec<_> = vertices.iter().map(|vertex| vertex.position).collect();
        let previous_positions: Vec<_> = vertices
            .iter()
            .map(|vertex| vertex.previous_position)
            .collect();
        let inverse_masses = anchors
            .iter()
            .map(|anchor| if anchor.is_some() { 0.0 } else { 1.0 })
            .collect();
        let (structural, bending) = build_constraints(&triangles, &positions);
        let uvs = vertices.iter().map(|vertex| vertex.uv).collect();
        let mut state = Self {
            positions,
            previous_positions,
            inverse_masses,
            anchors,
            triangles,
            structural,
            bending,
            uvs,
            settings: definition.settings,
        };
        state.enforce_anchors();
        Ok(state)
    }

    pub fn step(
        &mut self,
        dt: f32,
        colliders: &[ClothColliderSnapshot],
        vertices: &mut [Vertex],
    ) -> Result<(), ClothError> {
        if vertices.len() != self.positions.len() {
            return Err(ClothError::VertexBufferLength(
                self.positions.len(),
                vertices.len(),
            ));
        }
        self.settings.validate()?;
        if !dt.is_finite() || dt < 0.0 || !(dt * dt).is_finite() {
            return Err(ClothError::InvalidDeltaTime(dt));
        }
        for (index, collider) in colliders.iter().copied().enumerate() {
            collider.validate(index)?;
        }

        let acceleration = self.settings.gravity + self.settings.wind;
        let dt_squared = dt * dt;
        for index in 0..self.positions.len() {
            if let Some(anchor) = self.anchors[index] {
                self.positions[index] = anchor;
                self.previous_positions[index] = anchor;
            } else {
                let current = self.positions[index];
                let velocity = (current - self.previous_positions[index]) * self.settings.damping;
                self.previous_positions[index] = current;
                self.positions[index] = current + velocity + acceleration * dt_squared;
            }
        }

        for _ in 0..self.settings.constraint_iterations {
            for constraint in &self.structural {
                apply_constraint(
                    &mut self.positions,
                    &self.inverse_masses,
                    *constraint,
                    self.settings.structural_stiffness,
                );
            }
            for constraint in &self.bending {
                apply_constraint(
                    &mut self.positions,
                    &self.inverse_masses,
                    *constraint,
                    self.settings.bending_stiffness,
                );
            }
            self.enforce_anchors();
            for index in 0..self.positions.len() {
                if self.inverse_masses[index] == 0.0 {
                    continue;
                }
                for &collider in colliders {
                    resolve_collision(
                        &mut self.positions[index],
                        &mut self.previous_positions[index],
                        collider,
                        self.settings.particle_radius,
                        self.settings.collision_friction,
                    );
                }
            }
        }
        self.enforce_anchors();
        self.write_vertices(vertices);
        Ok(())
    }

    fn write_vertices(&self, vertices: &mut [Vertex]) {
        let (normals, tangents, signs) = self.compute_basis();
        for index in 0..self.positions.len() {
            vertices[index].previous_position = self.previous_positions[index];
            vertices[index].position = self.positions[index];
            vertices[index].normal = normals[index];
            vertices[index].tangent = tangents[index];
            vertices[index].tangent_sign = signs[index];
        }
    }

    fn enforce_anchors(&mut self) {
        for (index, anchor) in self.anchors.iter().enumerate() {
            if let Some(anchor) = anchor {
                self.positions[index] = *anchor;
                self.previous_positions[index] = *anchor;
            }
        }
    }

    fn compute_basis(&self) -> (Vec<Vec3>, Vec<Vec3>, Vec<f32>) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        let mut tangents = vec![Vec3::ZERO; self.positions.len()];
        let mut bitangents = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in &self.triangles {
            let ab = self.positions[b] - self.positions[a];
            let ac = self.positions[c] - self.positions[a];
            let face_normal = ab.cross(ac);
            if face_normal.dot(face_normal) <= EPSILON * EPSILON {
                continue;
            }
            for &index in &[a, b, c] {
                normals[index] += face_normal;
            }
            let uv_ab = self.uvs[b] - self.uvs[a];
            let uv_ac = self.uvs[c] - self.uvs[a];
            let determinant = uv_ab.x * uv_ac.y - uv_ab.y * uv_ac.x;
            let (tangent, bitangent) = if determinant.abs() > EPSILON {
                (
                    (ab * uv_ac.y - ac * uv_ab.y) * (1.0 / determinant),
                    (ac * uv_ab.x - ab * uv_ac.x) * (1.0 / determinant),
                )
            } else {
                (ab, Vec3::ZERO)
            };
            for &index in &[a, b, c] {
                tangents[index] += tangent;
                bitangents[index] += bitangent;
            }
        }

        let mut signs = vec![1.0; self.positions.len()];
        for index in 0..self.positions.len() {
            let normal = safe_normal(normals[index], Vec3::Y);
            let candidate = if tangents[index].dot(tangents[index]) > EPSILON * EPSILON {
                tangents[index]
            } else {
                Vec3::X
            };
            let tangent = safe_tangent(candidate, normal);
            let handedness = normal.cross(tangent).dot(bitangents[index]);
            signs[index] = if handedness.abs() > EPSILON {
                if handedness < 0.0 {
                    -1.0
                } else {
                    1.0
                }
            } else {
                1.0
            };
            normals[index] = normal;
            tangents[index] = tangent;
        }
        (normals, tangents, signs)
    }
}

fn build_constraints(
    triangles: &[[usize; 3]],
    positions: &[Vec3],
) -> (Vec<DistanceConstraint>, Vec<DistanceConstraint>) {
    let mut edges = BTreeSet::new();
    let mut opposites = BTreeMap::new();
    let mut bending_pairs = BTreeSet::new();
    for &[a, b, c] in triangles {
        for (first, second, opposite) in [(a, b, c), (b, c, a), (c, a, b)] {
            let edge = if first <= second {
                (first, second)
            } else {
                (second, first)
            };
            if edge.0 != edge.1 {
                edges.insert(edge);
            }
            if let Some(&other) = opposites.get(&edge) {
                let pair = if other <= opposite {
                    (other, opposite)
                } else {
                    (opposite, other)
                };
                if pair.0 != pair.1 {
                    bending_pairs.insert(pair);
                }
            } else {
                opposites.insert(edge, opposite);
            }
        }
    }
    let make = |(a, b)| DistanceConstraint {
        a,
        b,
        rest_length: (positions[b] - positions[a]).length(),
    };
    (
        edges.into_iter().map(make).collect(),
        bending_pairs.into_iter().map(make).collect(),
    )
}

fn apply_constraint(
    positions: &mut [Vec3],
    inverse_masses: &[f32],
    constraint: DistanceConstraint,
    stiffness: f32,
) {
    let sum = inverse_masses[constraint.a] + inverse_masses[constraint.b];
    if sum <= 0.0 || stiffness <= 0.0 {
        return;
    }
    let delta = positions[constraint.b] - positions[constraint.a];
    let length_squared = delta.dot(delta);
    if length_squared <= EPSILON * EPSILON || !length_squared.is_finite() {
        return;
    }
    let length = length_squared.sqrt();
    let correction = delta * ((length - constraint.rest_length) * stiffness / length);
    positions[constraint.a] += correction * (inverse_masses[constraint.a] / sum);
    positions[constraint.b] -= correction * (inverse_masses[constraint.b] / sum);
}

fn resolve_collision(
    position: &mut Vec3,
    previous_position: &mut Vec3,
    collider: ClothColliderSnapshot,
    particle_radius: f32,
    friction: f32,
) {
    let contact = match collider {
        ClothColliderSnapshot::Floor { height } => {
            let target = height + particle_radius;
            (position.y < target).then_some((Vec3::Y, target - position.y))
        }
        ClothColliderSnapshot::Sphere { center, radius } => {
            let delta = *position - center;
            let distance_squared = delta.dot(delta);
            let target = radius + particle_radius;
            if distance_squared >= target * target {
                None
            } else {
                let distance = distance_squared.sqrt();
                let normal = if distance > EPSILON {
                    delta / distance
                } else {
                    Vec3::Y
                };
                Some((normal, target - distance))
            }
        }
        ClothColliderSnapshot::Aabb { min, max } => {
            let min = Vec3::new(min.x.min(max.x), min.y.min(max.y), min.z.min(max.z));
            let max = Vec3::new(min.x.max(max.x), min.y.max(max.y), min.z.max(max.z));
            let closest = Vec3::new(
                position.x.clamp(min.x, max.x),
                position.y.clamp(min.y, max.y),
                position.z.clamp(min.z, max.z),
            );
            let delta = *position - closest;
            let distance_squared = delta.dot(delta);
            if distance_squared > EPSILON * EPSILON {
                let distance = distance_squared.sqrt();
                (distance < particle_radius)
                    .then_some((delta / distance, particle_radius - distance))
            } else {
                let faces = [
                    (position.x - min.x, -Vec3::X),
                    (max.x - position.x, Vec3::X),
                    (position.y - min.y, -Vec3::Y),
                    (max.y - position.y, Vec3::Y),
                    (position.z - min.z, -Vec3::Z),
                    (max.z - position.z, Vec3::Z),
                ];
                let (distance, normal) = faces
                    .into_iter()
                    .min_by(|left, right| left.0.total_cmp(&right.0))
                    .unwrap_or((0.0, Vec3::Y));
                Some((normal, particle_radius + distance))
            }
        }
    };
    let Some((normal, penetration)) = contact else {
        return;
    };
    if penetration <= 0.0 || !penetration.is_finite() {
        return;
    }
    *position += normal * penetration;
    let mut velocity = *position - *previous_position;
    let normal_speed = velocity.dot(normal);
    if normal_speed < 0.0 {
        velocity -= normal * normal_speed;
    }
    let normal_speed = velocity.dot(normal).max(0.0);
    let tangent = velocity - normal * velocity.dot(normal);
    *previous_position = *position - normal * normal_speed - tangent * (1.0 - friction);
}

fn finite(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn safe_normal(value: Vec3, fallback: Vec3) -> Vec3 {
    if finite(value) && value.dot(value) > EPSILON * EPSILON {
        value.normalize_or_zero()
    } else if finite(fallback) && fallback.dot(fallback) > EPSILON * EPSILON {
        fallback.normalize_or_zero()
    } else {
        Vec3::Y
    }
}

fn safe_tangent(value: Vec3, normal: Vec3) -> Vec3 {
    let projected = value - normal * normal.dot(value);
    if finite(projected) && projected.dot(projected) > EPSILON * EPSILON {
        return projected.normalize_or_zero();
    }
    let axis = if normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    axis.cross(normal).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Color, Vec2};

    fn vertex(position: Vec3, uv: Vec2) -> Vertex {
        Vertex {
            position,
            previous_position: position,
            normal: Vec3::Y,
            tangent: Vec3::X,
            tangent_sign: 1.0,
            uv,
            color: Color::WHITE,
        }
    }

    fn square() -> (Vec<Vertex>, Vec<[u32; 3]>) {
        (
            vec![
                vertex(Vec3::new(-1.0, 1.0, 0.0), Vec2::new(0.0, 0.0)),
                vertex(Vec3::new(1.0, 1.0, 0.0), Vec2::new(1.0, 0.0)),
                vertex(Vec3::new(1.0, -1.0, 0.0), Vec2::new(1.0, 1.0)),
                vertex(Vec3::new(-1.0, -1.0, 0.0), Vec2::new(0.0, 1.0)),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    #[test]
    fn pins_preserve_anchor_and_previous_position() {
        let (mut vertices, triangles) = square();
        let definition = ClothDefinition {
            pins: vec![ClothPin::at(0, Vec3::new(-2.0, 2.0, 0.0))],
            ..ClothDefinition::default()
        };
        let mut state = ClothState::new(&vertices, &triangles, &definition).unwrap();
        state.step(1.0 / 60.0, &[], &mut vertices).unwrap();
        assert_eq!(state.positions[0], Vec3::new(-2.0, 2.0, 0.0));
        assert_eq!(vertices[0].position, state.positions[0]);
        assert_eq!(vertices[0].previous_position, state.previous_positions[0]);
    }

    #[test]
    fn constraints_are_unique_and_deterministic() {
        let (vertices, triangles) = square();
        let mut definition = ClothDefinition::default();
        definition.settings.gravity = Vec3::ZERO;
        definition.settings.damping = 1.0;
        definition.settings.constraint_iterations = 4;
        let mut left = ClothState::new(&vertices, &triangles, &definition).unwrap();
        let mut right = left.clone();
        assert_eq!(left.structural.len(), 5);
        assert_eq!(left.bending.len(), 1);
        left.positions[2] = Vec3::new(1.4, -1.2, 0.3);
        right.positions[2] = Vec3::new(1.4, -1.2, 0.3);
        let mut left_vertices = vertices.clone();
        let mut right_vertices = vertices;
        left.step(1.0 / 60.0, &[], &mut left_vertices).unwrap();
        right.step(1.0 / 60.0, &[], &mut right_vertices).unwrap();
        assert_eq!(left.positions, right.positions);
        assert_eq!(left_vertices, right_vertices);
    }

    #[test]
    fn floor_sphere_and_aabb_collisions_project_particles() {
        let mut vertices = vec![vertex(Vec3::new(0.0, -2.0, 0.0), Vec2::ZERO)];
        let triangles = Vec::new();
        let mut definition = ClothDefinition::default();
        definition.settings.gravity = Vec3::ZERO;
        definition.settings.particle_radius = 0.1;
        let mut state = ClothState::new(&vertices, &triangles, &definition).unwrap();
        state
            .step(0.0, &[ClothColliderSnapshot::floor(0.0)], &mut vertices)
            .unwrap();
        assert!(state.positions[0].y >= 0.1 - EPSILON);
        vertices[0].position = Vec3::ZERO;
        vertices[0].previous_position = Vec3::ZERO;
        let mut state = ClothState::new(&vertices, &triangles, &definition).unwrap();
        state
            .step(
                0.0,
                &[ClothColliderSnapshot::sphere(Vec3::ZERO, 1.0)],
                &mut vertices,
            )
            .unwrap();
        assert!(state.positions[0].length() >= 1.1 - EPSILON);
        vertices[0].position = Vec3::ZERO;
        vertices[0].previous_position = Vec3::ZERO;
        let mut state = ClothState::new(&vertices, &triangles, &definition).unwrap();
        state
            .step(
                0.0,
                &[ClothColliderSnapshot::aabb(
                    Vec3::new(-1.0, -1.0, -1.0),
                    Vec3::new(1.0, 1.0, 1.0),
                )],
                &mut vertices,
            )
            .unwrap();
        assert!(state.positions[0].x.abs() >= 1.1 - EPSILON);
    }

    #[test]
    fn recomputed_basis_is_finite_for_degenerate_uvs() {
        let mut vertices = vec![
            vertex(Vec3::new(0.0, 0.0, 0.0), Vec2::ZERO),
            vertex(Vec3::new(1.0, 0.0, 0.0), Vec2::ZERO),
            vertex(Vec3::new(0.0, 0.0, 1.0), Vec2::ZERO),
        ];
        let triangles = vec![[0, 1, 2]];
        let mut state =
            ClothState::new(&vertices, &triangles, &ClothDefinition::default()).unwrap();
        state.step(0.0, &[], &mut vertices).unwrap();
        for vertex in vertices {
            assert!(finite(vertex.position));
            assert!(finite(vertex.previous_position));
            assert!(finite(vertex.normal));
            assert!(finite(vertex.tangent));
            assert!(vertex.tangent_sign.is_finite());
            assert!((vertex.normal.length() - 1.0).abs() < 1.0e-5);
            assert!((vertex.tangent.length() - 1.0).abs() < 1.0e-5);
        }
    }
}
