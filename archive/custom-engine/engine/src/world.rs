use crate::asset::ShadowCasterClass;
use crate::cloth::{ClothColliderSnapshot, ClothDefinition, ClothError, ClothState};
use crate::math::{Color, Mat4, Vec3};
use crate::scene::{
    Camera, EnvironmentMap, EnvironmentProfile, Light, LightKind, Material, Mesh, RenderScene,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityId {
    index: u32,
    generation: u32,
}

impl EntityId {
    pub const fn raw_parts(self) -> (u32, u32) {
        (self.index, self.generation)
    }
}

impl Hash for EntityId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

#[derive(Clone, Debug)]
struct EntitySlot {
    generation: u32,
    alive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Name(pub String);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub previous_position: Vec3,
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        previous_position: Vec3::ZERO,
        position: Vec3::ZERO,
        rotation: Vec3::ZERO,
        scale: Vec3::ONE,
    };

    pub fn from_position(position: Vec3) -> Self {
        Self {
            previous_position: position,
            position,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }

    pub fn matrix(self) -> Mat4 {
        Mat4::translation(self.position)
            * Mat4::rotation_y(self.rotation.y)
            * Mat4::rotation_x(self.rotation.x)
            * Mat4::rotation_z(self.rotation.z)
            * Mat4::scale(self.scale)
    }

    pub fn velocity(self, dt: f32) -> Vec3 {
        if dt > 0.0 {
            (self.position - self.previous_position) / dt
        } else {
            Vec3::ZERO
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshRenderer {
    pub mesh: Mesh,
    pub visible: bool,
    pub prepass_eligible: bool,
    pub reflection_proxy: bool,
}

/// A visible mesh as the live renderer streams it, borrowed from the world.
#[derive(Clone, Copy, Debug)]
pub struct RenderInstance<'a> {
    pub entity: EntityId,
    pub mesh: &'a Mesh,
    /// Mesh-local to world transform (mesh transform, then entity transform).
    pub transform: Mat4,
    /// Changes whenever the mesh geometry or renderer settings are replaced.
    pub revision: u64,
    /// Geometry deforms every tick (cloth) even when the transform is static.
    pub deforming: bool,
    pub prepass_eligible: bool,
    pub reflection_proxy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraRig {
    pub camera: Camera,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightComponent {
    pub kind: LightKind,
    pub color: Color,
    pub intensity: f32,
    pub casts_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidBody {
    pub velocity: Vec3,
    pub mass: f32,
    pub gravity_scale: f32,
    pub dynamic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsSettings {
    pub gravity: Vec3,
    pub floor_height: Option<f32>,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            floor_height: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    Sphere { radius: f32 },
    Aabb { half_extents: Vec3 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptState {
    pub module: String,
    pub enabled: bool,
    pub ticks: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorldIssue {
    MissingActiveCamera,
    MeshMaterialOutOfRange {
        entity: EntityId,
        mesh: String,
        material_index: usize,
    },
    FloorCastsShadow {
        entity: EntityId,
        mesh: String,
    },
    ReflectiveMeshMissingProxy {
        entity: EntityId,
        mesh: String,
    },
}

#[derive(Clone, Debug)]
pub struct World {
    slots: Vec<EntitySlot>,
    free: Vec<u32>,
    names: HashMap<EntityId, Name>,
    transforms: HashMap<EntityId, Transform>,
    mesh_renderers: HashMap<EntityId, MeshRenderer>,
    mesh_revisions: HashMap<EntityId, u64>,
    next_mesh_revision: u64,
    cameras: HashMap<EntityId, CameraRig>,
    lights: HashMap<EntityId, LightComponent>,
    rigid_bodies: HashMap<EntityId, RigidBody>,
    cloths: HashMap<EntityId, ClothState>,
    colliders: HashMap<EntityId, Collider>,
    scripts: HashMap<EntityId, ScriptState>,
    pub physics: PhysicsSettings,
    pub materials: Vec<Material>,
    pub ambient_color: Color,
    pub environment_map: EnvironmentMap,
    pub environment: EnvironmentProfile,
    tick: u64,
}

impl Default for World {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            names: HashMap::new(),
            transforms: HashMap::new(),
            mesh_renderers: HashMap::new(),
            mesh_revisions: HashMap::new(),
            next_mesh_revision: 1,
            cameras: HashMap::new(),
            lights: HashMap::new(),
            rigid_bodies: HashMap::new(),
            cloths: HashMap::new(),
            colliders: HashMap::new(),
            scripts: HashMap::new(),
            physics: PhysicsSettings::default(),
            materials: Vec::new(),
            ambient_color: Color::rgb(0.035, 0.045, 0.055),
            environment_map: EnvironmentMap::solid(Color::rgb(0.08, 0.12, 0.18)),
            environment: EnvironmentProfile::rainy(),
            tick: 0,
        }
    }
}

impl World {
    pub fn spawn(&mut self, name: impl Into<String>, transform: Transform) -> EntityId {
        let id = if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.alive = true;
            EntityId {
                index,
                generation: slot.generation,
            }
        } else {
            let index = self.slots.len() as u32;
            self.slots.push(EntitySlot {
                generation: 1,
                alive: true,
            });
            EntityId {
                index,
                generation: 1,
            }
        };
        self.names.insert(id, Name(name.into()));
        self.transforms.insert(id, transform);
        id
    }

    pub fn destroy(&mut self, id: EntityId) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        let slot = &mut self.slots[id.index as usize];
        slot.alive = false;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(id.index);
        self.names.remove(&id);
        self.transforms.remove(&id);
        self.mesh_renderers.remove(&id);
        self.mesh_revisions.remove(&id);
        self.cameras.remove(&id);
        self.lights.remove(&id);
        self.rigid_bodies.remove(&id);
        self.cloths.remove(&id);
        self.colliders.remove(&id);
        self.scripts.remove(&id);
        true
    }

    pub fn is_alive(&self, id: EntityId) -> bool {
        self.slots
            .get(id.index as usize)
            .is_some_and(|slot| slot.alive && slot.generation == id.generation)
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn set_mesh_renderer(&mut self, id: EntityId, mut renderer: MeshRenderer) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        if renderer.mesh.is_floor_or_ground {
            renderer.mesh.shadow_class = ShadowCasterClass::None;
        }
        renderer.mesh.prepass_eligible = renderer.prepass_eligible;
        renderer.mesh.reflection_proxy = renderer.reflection_proxy;
        self.mesh_renderers.insert(id, renderer);
        self.bump_mesh_revision(id);
        true
    }

    /// Mutable access to a mesh renderer. The live renderer treats every call as
    /// a geometry change, so prefer moving entities through their [`Transform`].
    pub fn mesh_renderer_mut(&mut self, id: EntityId) -> Option<&mut MeshRenderer> {
        if !self.is_alive(id) || !self.mesh_renderers.contains_key(&id) {
            return None;
        }
        self.bump_mesh_revision(id);
        self.mesh_renderers.get_mut(&id)
    }

    fn bump_mesh_revision(&mut self, id: EntityId) {
        self.mesh_revisions.insert(id, self.next_mesh_revision);
        self.next_mesh_revision += 1;
    }

    pub fn set_camera(&mut self, id: EntityId, camera: CameraRig) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.cameras.insert(id, camera);
        true
    }

    pub fn set_light(&mut self, id: EntityId, light: LightComponent) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.lights.insert(id, light);
        true
    }

    pub fn set_rigid_body(&mut self, id: EntityId, body: RigidBody) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.rigid_bodies.insert(id, body);
        true
    }

    pub fn set_collider(&mut self, id: EntityId, collider: Collider) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.colliders.insert(id, collider);
        true
    }

    pub fn set_cloth(
        &mut self,
        id: EntityId,
        definition: &ClothDefinition,
    ) -> Result<bool, ClothError> {
        if !self.is_alive(id) {
            return Ok(false);
        }
        let Some(renderer) = self.mesh_renderers.get_mut(&id) else {
            return Ok(false);
        };
        renderer.mesh.bake_transform();
        let state = ClothState::new(&renderer.mesh.vertices, &renderer.mesh.indices, definition)?;
        renderer.mesh.cloth = Some(definition.clone());
        self.cloths.insert(id, state);
        Ok(true)
    }

    pub fn set_script(&mut self, id: EntityId, script: ScriptState) -> bool {
        if !self.is_alive(id) {
            return false;
        }
        self.scripts.insert(id, script);
        true
    }

    pub fn transform(&self, id: EntityId) -> Option<&Transform> {
        self.transforms.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn transform_mut(&mut self, id: EntityId) -> Option<&mut Transform> {
        if self.is_alive(id) {
            self.transforms.get_mut(&id)
        } else {
            None
        }
    }

    pub fn entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.alive.then_some(EntityId {
                index: index as u32,
                generation: slot.generation,
            })
        })
    }

    pub fn name(&self, id: EntityId) -> Option<&str> {
        self.names
            .get(&id)
            .filter(|_| self.is_alive(id))
            .map(|name| name.0.as_str())
    }

    pub fn find_by_name(&self, name: &str) -> Option<EntityId> {
        self.names
            .iter()
            .find_map(|(id, candidate)| (self.is_alive(*id) && candidate.0 == name).then_some(*id))
    }

    pub fn camera(&self, id: EntityId) -> Option<&CameraRig> {
        self.cameras.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn camera_mut(&mut self, id: EntityId) -> Option<&mut CameraRig> {
        if self.is_alive(id) {
            self.cameras.get_mut(&id)
        } else {
            None
        }
    }

    pub fn active_camera(&self) -> Option<(EntityId, &CameraRig)> {
        self.cameras
            .iter()
            .find(|(id, camera)| self.is_alive(**id) && camera.active)
            .map(|(id, camera)| (*id, camera))
    }

    pub fn active_camera_mut(&mut self) -> Option<(EntityId, &mut CameraRig)> {
        let id = self
            .cameras
            .iter()
            .find_map(|(id, camera)| (self.is_alive(*id) && camera.active).then_some(*id))?;
        Some((id, self.cameras.get_mut(&id).expect("active camera exists")))
    }

    pub fn set_active_camera(&mut self, id: EntityId) -> bool {
        if !self.is_alive(id) || !self.cameras.contains_key(&id) {
            return false;
        }
        for camera in self.cameras.values_mut() {
            camera.active = false;
        }
        self.cameras
            .get_mut(&id)
            .expect("camera checked above")
            .active = true;
        true
    }

    pub fn rigid_body(&self, id: EntityId) -> Option<&RigidBody> {
        self.rigid_bodies.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn rigid_body_mut(&mut self, id: EntityId) -> Option<&mut RigidBody> {
        if self.is_alive(id) {
            self.rigid_bodies.get_mut(&id)
        } else {
            None
        }
    }

    pub fn collider(&self, id: EntityId) -> Option<&Collider> {
        self.colliders.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn script(&self, id: EntityId) -> Option<&ScriptState> {
        self.scripts.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn script_mut(&mut self, id: EntityId) -> Option<&mut ScriptState> {
        if self.is_alive(id) {
            self.scripts.get_mut(&id)
        } else {
            None
        }
    }

    pub fn mesh_renderer(&self, id: EntityId) -> Option<&MeshRenderer> {
        self.mesh_renderers.get(&id).filter(|_| self.is_alive(id))
    }

    pub fn entity_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.alive).count()
    }

    pub fn fixed_update(&mut self, dt: f32) -> Result<(), ClothError> {
        let dt = dt.max(0.0);
        let physics = self.physics;
        let ids: Vec<EntityId> = self.rigid_bodies.keys().copied().collect();
        for id in ids {
            if !self.is_alive(id) {
                continue;
            }
            let Some(body) = self.rigid_bodies.get_mut(&id) else {
                continue;
            };
            let Some(transform) = self.transforms.get_mut(&id) else {
                continue;
            };
            transform.previous_position = transform.position;
            if body.dynamic {
                body.velocity += physics.gravity * (body.gravity_scale * dt);
                transform.position += body.velocity * dt;
                if let Some(floor) = physics.floor_height {
                    if transform.position.y < floor {
                        transform.position.y = floor;
                        if body.velocity.y < 0.0 {
                            body.velocity.y = 0.0;
                        }
                    }
                }
            }
        }
        for script in self.scripts.values_mut() {
            if script.enabled {
                script.ticks = script.ticks.saturating_add(1);
            }
        }
        let mut collider_ids: Vec<_> = self.colliders.keys().copied().collect();
        collider_ids.sort_unstable();
        let mut cloth_colliders = Vec::new();
        if let Some(floor) = physics.floor_height {
            cloth_colliders.push(ClothColliderSnapshot::floor(floor));
        }
        for id in collider_ids {
            let position = self
                .transforms
                .get(&id)
                .map(|transform| transform.position)
                .unwrap_or(Vec3::ZERO);
            match self.colliders[&id] {
                Collider::Sphere { radius } => {
                    cloth_colliders.push(ClothColliderSnapshot::sphere(position, radius));
                }
                Collider::Aabb { half_extents } => {
                    cloth_colliders.push(ClothColliderSnapshot::aabb(
                        position - half_extents,
                        position + half_extents,
                    ));
                }
            }
        }
        let mut cloth_ids: Vec<_> = self.cloths.keys().copied().collect();
        cloth_ids.sort_unstable();
        for id in cloth_ids {
            let state = self.cloths.get_mut(&id).expect("cloth id came from map");
            if let Some(renderer) = self.mesh_renderers.get_mut(&id) {
                state.step(dt, &cloth_colliders, &mut renderer.mesh.vertices)?;
            }
        }
        self.tick = self.tick.saturating_add(1);
        Ok(())
    }

    /// Visible meshes in entity order, without copying geometry.
    pub fn render_instances(&self) -> Vec<RenderInstance<'_>> {
        let mut entities: Vec<_> = self
            .mesh_renderers
            .iter()
            .filter(|(entity, renderer)| self.is_alive(**entity) && renderer.visible)
            .map(|(entity, _)| *entity)
            .collect();
        entities.sort_unstable();
        entities
            .into_iter()
            .map(|entity| {
                let renderer = &self.mesh_renderers[&entity];
                let transform = match self.transforms.get(&entity) {
                    Some(transform) => transform.matrix() * renderer.mesh.transform,
                    None => renderer.mesh.transform,
                };
                RenderInstance {
                    entity,
                    mesh: &renderer.mesh,
                    transform,
                    revision: self.mesh_revisions.get(&entity).copied().unwrap_or(0),
                    deforming: self.cloths.contains_key(&entity),
                    prepass_eligible: renderer.prepass_eligible,
                    reflection_proxy: renderer.reflection_proxy,
                }
            })
            .collect()
    }

    /// Camera, lights and environment for this frame. Meshes and materials are
    /// left empty; use [`World::render_instances`] and [`World::materials`].
    pub fn build_lighting_scene(&self) -> RenderScene {
        RenderScene {
            camera: self.render_camera(),
            materials: Vec::new(),
            meshes: Vec::new(),
            lights: self.render_lights(),
            ambient_color: self.ambient_color,
            environment_map: self.environment_map.clone(),
            environment: self.environment,
        }
    }

    fn render_camera(&self) -> Camera {
        self.cameras
            .iter()
            .find_map(|(id, rig)| (self.is_alive(*id) && rig.active).then_some(rig.camera))
            .unwrap_or(Camera {
                position: Vec3::new(0.0, 1.2, 4.0),
                target: Vec3::ZERO,
                up: Vec3::Y,
                vertical_fov_radians: 55.0_f32.to_radians(),
                near: 0.05,
                far: 100.0,
            })
    }

    fn render_lights(&self) -> Vec<Light> {
        let mut light_entities: Vec<_> = self.lights.keys().copied().collect();
        light_entities.sort_unstable();
        light_entities
            .into_iter()
            .filter(|entity| self.is_alive(*entity))
            .map(|entity| {
                let component = &self.lights[&entity];
                let position = self
                    .transforms
                    .get(&entity)
                    .map(|transform| transform.position)
                    .unwrap_or(Vec3::ZERO);
                Light {
                    name: self
                        .names
                        .get(&entity)
                        .map(|name| name.0.clone())
                        .unwrap_or_else(|| "world_light".to_string()),
                    kind: component.kind.at_position(position),
                    color: component.color,
                    intensity: component.intensity,
                    casts_shadow: component.casts_shadow,
                }
            })
            .collect()
    }

    pub fn validate(&self) -> Vec<WorldIssue> {
        let mut issues = Vec::new();
        if !self.cameras.values().any(|camera| camera.active) {
            issues.push(WorldIssue::MissingActiveCamera);
        }
        for (entity, renderer) in &self.mesh_renderers {
            if renderer.mesh.material_index >= self.materials.len() {
                issues.push(WorldIssue::MeshMaterialOutOfRange {
                    entity: *entity,
                    mesh: renderer.mesh.name.clone(),
                    material_index: renderer.mesh.material_index,
                });
            }
            if renderer.mesh.is_floor_or_ground
                && renderer.mesh.shadow_class != ShadowCasterClass::None
            {
                issues.push(WorldIssue::FloorCastsShadow {
                    entity: *entity,
                    mesh: renderer.mesh.name.clone(),
                });
            }
            if let Some(material) = self.materials.get(renderer.mesh.material_index) {
                if (material.flags.glass || material.specular_intensity > 0.75)
                    && !renderer.reflection_proxy
                {
                    issues.push(WorldIssue::ReflectiveMeshMissingProxy {
                        entity: *entity,
                        mesh: renderer.mesh.name.clone(),
                    });
                }
            }
        }
        issues
    }

    pub fn build_render_scene(&self) -> RenderScene {
        let meshes = self
            .render_instances()
            .into_iter()
            .map(|instance| {
                let mut mesh = instance.mesh.clone();
                mesh.transform = instance.transform;
                mesh.prepass_eligible = instance.prepass_eligible;
                mesh.reflection_proxy = instance.reflection_proxy;
                if mesh.is_floor_or_ground {
                    mesh.shadow_class = ShadowCasterClass::None;
                }
                mesh
            })
            .collect();
        RenderScene {
            meshes,
            materials: self.materials.clone(),
            ..self.build_lighting_scene()
        }
    }

    pub fn into_render_scene(self) -> RenderScene {
        let Self {
            slots,
            transforms,
            mesh_renderers,
            cameras,
            lights: light_components,
            names,
            materials,
            ambient_color,
            environment_map,
            environment,
            ..
        } = self;
        let is_alive = |id: EntityId| {
            slots
                .get(id.index as usize)
                .is_some_and(|slot| slot.alive && slot.generation == id.generation)
        };
        let camera = cameras
            .iter()
            .find_map(|(id, rig)| (is_alive(*id) && rig.active).then_some(rig.camera))
            .unwrap_or(Camera {
                position: Vec3::new(0.0, 1.2, 4.0),
                target: Vec3::ZERO,
                up: Vec3::Y,
                vertical_fov_radians: 55.0_f32.to_radians(),
                near: 0.05,
                far: 100.0,
            });

        let meshes = mesh_renderers
            .into_iter()
            .filter_map(|(entity, renderer)| {
                if !is_alive(entity) || !renderer.visible {
                    return None;
                }
                let mut mesh = renderer.mesh;
                if let Some(transform) = transforms.get(&entity) {
                    mesh.transform = transform.matrix() * mesh.transform;
                }
                mesh.prepass_eligible = renderer.prepass_eligible;
                mesh.reflection_proxy = renderer.reflection_proxy;
                if mesh.is_floor_or_ground {
                    mesh.shadow_class = ShadowCasterClass::None;
                }
                Some(mesh)
            })
            .collect();

        let lights = light_components
            .into_iter()
            .filter_map(|(entity, component)| {
                if !is_alive(entity) {
                    return None;
                }
                let kind = match component.kind {
                    LightKind::Directional { direction } => LightKind::Directional { direction },
                    LightKind::Local { radius, .. } => LightKind::Local {
                        position: transforms
                            .get(&entity)
                            .map(|transform| transform.position)
                            .unwrap_or(Vec3::ZERO),
                        radius,
                    },
                    LightKind::Spot {
                        direction,
                        radius,
                        inner_angle_radians,
                        outer_angle_radians,
                        ..
                    } => LightKind::Spot {
                        position: transforms
                            .get(&entity)
                            .map(|transform| transform.position)
                            .unwrap_or(Vec3::ZERO),
                        direction,
                        radius,
                        inner_angle_radians,
                        outer_angle_radians,
                    },
                };
                Some(Light {
                    name: names
                        .get(&entity)
                        .map(|name| name.0.clone())
                        .unwrap_or_else(|| "world_light".to_string()),
                    kind,
                    color: component.color,
                    intensity: component.intensity,
                    casts_shadow: component.casts_shadow,
                })
            })
            .collect();

        RenderScene {
            camera,
            materials,
            meshes,
            lights,
            ambient_color,
            environment_map,
            environment,
        }
    }

    pub fn from_render_scene(scene: RenderScene) -> Self {
        let mut world = Self {
            materials: scene.materials,
            ambient_color: scene.ambient_color,
            environment_map: scene.environment_map,
            environment: scene.environment,
            ..Self::default()
        };
        let camera_entity = world.spawn(
            "Main Camera",
            Transform::from_position(scene.camera.position),
        );
        world.set_camera(
            camera_entity,
            CameraRig {
                camera: scene.camera,
                active: true,
            },
        );
        for mut mesh in scene.meshes {
            let cloth = mesh.cloth.clone();
            if cloth.is_some() {
                mesh.bake_transform();
            }
            let entity = world.spawn(mesh.name.clone(), Transform::IDENTITY);
            let prepass_eligible = mesh.prepass_eligible;
            let reflection_proxy = mesh.reflection_proxy;
            world.set_mesh_renderer(
                entity,
                MeshRenderer {
                    mesh,
                    visible: true,
                    prepass_eligible,
                    reflection_proxy,
                },
            );
            if let Some(definition) = cloth {
                world
                    .set_cloth(entity, &definition)
                    .expect("scene cloth definition must be valid");
            }
        }
        for light in scene.lights {
            let position = match light.kind {
                LightKind::Directional { .. } => Vec3::ZERO,
                LightKind::Local { position, .. } => position,
                LightKind::Spot { position, .. } => position,
            };
            let entity = world.spawn(light.name, Transform::from_position(position));
            world.set_light(
                entity,
                LightComponent {
                    kind: light.kind,
                    color: light.color,
                    intensity: light.intensity,
                    casts_shadow: light.casts_shadow,
                },
            );
        }
        world
    }
}
