//! Live cockpit steering wheel: the RB22 LCD shows an egui-painted dash and
//! its LED strip lights per lamp, both driven by the player's car. Wheels
//! modelled without a display (the RB6) get a slim screen pod on top.

use crate::{assets::SteeringWheelVisual, hud, Session};
use bevy::{
    asset::RenderAssetUsages,
    camera::{visibility::RenderLayers, RenderTarget},
    ecs::schedule::ScheduleLabel,
    math::Affine2,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
    render::render_resource::TextureFormat,
};
use bevy_egui::{egui, EguiContext, EguiSchedule, PrimaryEguiContext};

/// Imported LCD and LED-strip primitives on a car's wheel; only the player's
/// are made live, AI wheels keep their imported look.
#[derive(Component)]
pub enum WheelPart {
    Screen(usize),
    Leds(usize),
    /// A wheel rim with no display of its own.
    Mount(usize),
}

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WheelDashPass;

#[derive(Resource)]
pub struct WheelDash {
    image: Handle<Image>,
    /// The player's wheel uses the pod's strip layout instead of the full LCD.
    compact: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lamp {
    Rev(usize),
    /// Left column, bottom to top: grip warnings.
    Left(usize),
    /// Right column, bottom to top: tow strength.
    Right(usize),
}

#[derive(Component)]
pub struct WheelLamp {
    lamp: Lamp,
    lit: Option<Color>,
    off: Color,
}

const REV_LAMPS: usize = 15;

pub fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let (width, height) = hud::wheel::SIZE;
    let image = images.add(Image::new_target_texture(
        width,
        height,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    // Draws nothing of the world; it only hosts the wheel's egui context.
    commands.spawn((
        Camera2d,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        RenderLayers::none(),
        EguiSchedule::new(WheelDashPass),
    ));
    commands.insert_resource(WheelDash {
        image,
        compact: false,
    });
}

pub fn draw(
    mut context: Single<&mut EguiContext, Without<PrimaryEguiContext>>,
    style: Res<hud::HudStyle>,
    session: Res<Session>,
    dash: Res<WheelDash>,
    mut initialized: Local<bool>,
) {
    let context = context.get_mut();
    if !*initialized {
        context.set_fonts(hud::fonts());
        *initialized = true;
        return;
    }
    if !session.ready {
        return;
    }
    let canvas = crate::canvas::Hud {
        painter: context.layer_painter(egui::LayerId::background()),
        size: context.content_rect().size(),
        finish: crate::canvas::Finish::Digital,
    };
    if dash.compact {
        hud::wheel::draw_compact(&canvas, &style, &session);
    } else {
        hud::wheel::draw(&canvas, &style, &session);
    }
}

type WheelPartItem = (
    Entity,
    &'static WheelPart,
    &'static Mesh3d,
    &'static MeshMaterial3d<StandardMaterial>,
    &'static Transform,
    &'static SteeringWheelVisual,
    &'static ChildOf,
);

/// Moves the player's LCD onto the live texture and splits its LED strip into
/// individually lit lamps.
pub fn attach(
    mut commands: Commands,
    parts: Query<WheelPartItem>,
    session: Res<Session>,
    mut dash: ResMut<WheelDash>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, part, mesh, material, transform, visual, parent) in &parts {
        commands.entity(entity).remove::<WheelPart>();
        let (WheelPart::Screen(entrant) | WheelPart::Leds(entrant) | WheelPart::Mount(entrant)) =
            *part;
        if entrant != session.player {
            continue;
        }
        let Some(source) = meshes.get(&mesh.0).cloned() else {
            continue;
        };
        match part {
            WheelPart::Mount(_) => {
                dash.compact = true;
                let mut pod = |mesh: Mesh, material: StandardMaterial| {
                    (
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(materials.add(material)),
                        *transform,
                        SteeringWheelVisual {
                            entrant,
                            rest: visual.rest,
                            pivot: visual.pivot,
                        },
                        ChildOf(parent.parent()),
                    )
                };
                // Sits in the open slot above the button plate, whose
                // driver-facing surface is at z ≈ 0.685 in the RB6 model.
                let housing = pod(
                    Cuboid::new(0.124, 0.032, 0.012)
                        .mesh()
                        .build()
                        .translated_by(Vec3::new(0., 0.597, 0.6865)),
                    StandardMaterial {
                        base_color: Color::srgb(0.03, 0.03, 0.035),
                        perceptual_roughness: 0.45,
                        ..default()
                    },
                );
                let screen = pod(
                    screen_quad(Vec2::new(0.108, 0.022), Vec3::new(0., 0.597, 0.6802)),
                    StandardMaterial {
                        base_color_texture: Some(dash.image.clone()),
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        uv_transform: Affine2::from_scale(Vec2::new(
                            1.,
                            hud::wheel::COMPACT_HEIGHT / hud::wheel::SIZE.1 as f32,
                        )),
                        ..default()
                    },
                );
                commands.spawn(housing);
                commands.spawn(screen);
            }
            WheelPart::Screen(_) => {
                dash.compact = false;
                let Some(window) = screen_uvs(&source) else {
                    continue;
                };
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(materials.add(StandardMaterial {
                        base_color_texture: Some(dash.image.clone()),
                        unlit: true,
                        uv_transform: Affine2::from_scale(Vec2::ONE / window.size())
                            * Affine2::from_translation(-window.min),
                        ..default()
                    })));
            }
            WheelPart::Leds(_) => {
                let Some(base) = materials.get(&material.0).cloned() else {
                    continue;
                };
                let (lamps, rest) = split_lamps(&source);
                for (lamp, indices) in lamps {
                    let mut lamp_mesh = source.clone();
                    lamp_mesh.insert_indices(Indices::U32(indices));
                    commands.spawn((
                        Mesh3d(meshes.add(lamp_mesh)),
                        MeshMaterial3d(materials.add(base.clone())),
                        *transform,
                        SteeringWheelVisual {
                            entrant,
                            rest: visual.rest,
                            pivot: visual.pivot,
                        },
                        WheelLamp {
                            lamp,
                            lit: None,
                            off: base.base_color,
                        },
                        ChildOf(parent.parent()),
                    ));
                }
                let mut rest_mesh = source;
                rest_mesh.insert_indices(Indices::U32(rest));
                commands
                    .entity(entity)
                    .insert(Mesh3d(meshes.add(rest_mesh)));
            }
        }
    }
}

/// Driver-facing quad (normal −Z) whose texture's left edge is the driver's
/// left, +X in car space.
fn screen_quad(size: Vec2, centre: Vec3) -> Mesh {
    let (x, y) = (size.x * 0.5, size.y * 0.5);
    let corners = [(x, y), (-x, y), (-x, -y), (x, -y)];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        corners
            .iter()
            .map(|&(dx, dy)| [centre.x + dx, centre.y + dy, centre.z])
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., -1.]; 4])
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 2, 3, 0]))
}

/// The LCD quad is two-sided; its driver-facing side uses non-negative V.
fn screen_uvs(mesh: &Mesh) -> Option<Rect> {
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        return None;
    };
    let mut window = Rect::EMPTY;
    for uv in uvs.iter().filter(|uv| uv[1] >= 0.) {
        window = window.union_point(Vec2::from_array(*uv));
    }
    (window.width() > 0. && window.height() > 0.).then_some(window)
}

/// Groups the strip's triangles into lamps by connected piece: the row above
/// the screen is the rev counter, the columns either side are status lamps.
fn split_lamps(mesh: &Mesh) -> (Vec<(Lamp, Vec<u32>)>, Vec<u32>) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return (Vec::new(), Vec::new());
    };
    let indices: Vec<u32> = match mesh.indices() {
        Some(indices) => indices.iter().map(|index| index as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    let mut parent: Vec<usize> = (0..positions.len()).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    for triangle in indices.chunks_exact(3) {
        let a = root(&mut parent, triangle[0] as usize);
        for &other in &triangle[1..] {
            let b = root(&mut parent, other as usize);
            parent[b] = a;
        }
    }
    // Centroid per piece.
    let mut sums = vec![(Vec3::ZERO, 0.); positions.len()];
    for (index, position) in positions.iter().enumerate() {
        let piece = root(&mut parent, index);
        sums[piece].0 += Vec3::from_array(*position);
        sums[piece].1 += 1.;
    }
    let centroid = |piece: usize| sums[piece].0 / sums[piece].1;
    let top = positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
    // Pieces at the same lamp position (lens and housing) share one lamp.
    let mut rev_x: Vec<f32> = Vec::new();
    let mut side: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
    let mut pieces = Vec::new();
    for piece in 0..positions.len() {
        if root(&mut parent, piece) != piece {
            continue;
        }
        let c = centroid(piece);
        pieces.push((piece, c));
        if c.y > top - 0.006 {
            rev_x.push(c.x);
        } else if (0.045..0.065).contains(&c.x.abs()) {
            side[usize::from(c.x > 0.)].push(c.y);
        }
    }
    // Lamps sit 3–5 mm apart; pieces of one lamp are within a millimetre.
    let cluster = |values: &mut Vec<f32>| {
        values.sort_by(f32::total_cmp);
        let mut centres: Vec<(f32, f32)> = Vec::new();
        let mut last = f32::MIN;
        for &value in values.iter() {
            match centres.last_mut() {
                Some((sum, count)) if value - last < 0.002 => {
                    *sum += value;
                    *count += 1.;
                }
                _ => centres.push((value, 1.)),
            }
            last = value;
        }
        *values = centres.iter().map(|(sum, count)| sum / count).collect();
    };
    cluster(&mut rev_x);
    cluster(&mut side[0]);
    cluster(&mut side[1]);
    let nearest = |values: &[f32], value: f32| {
        (0..values.len())
            .min_by(|&a, &b| {
                (values[a] - value)
                    .abs()
                    .total_cmp(&(values[b] - value).abs())
            })
            .unwrap_or(0)
    };
    let lamp_of = |c: Vec3| {
        if c.y > top - 0.006 && rev_x.len() == REV_LAMPS {
            // The driver looks along +Z, so their left is +X.
            Some(Lamp::Rev(REV_LAMPS - 1 - nearest(&rev_x, c.x)))
        } else if (0.045..0.065).contains(&c.x.abs()) {
            let column = &side[usize::from(c.x > 0.)];
            let row = nearest(column, c.y);
            Some(if c.x > 0. {
                Lamp::Left(row)
            } else {
                Lamp::Right(row)
            })
        } else {
            None
        }
    };
    let mut lamps: Vec<(Lamp, Vec<u32>)> = Vec::new();
    let mut rest = Vec::new();
    for triangle in indices.chunks_exact(3) {
        let piece = root(&mut parent, triangle[0] as usize);
        match lamp_of(centroid(piece)) {
            Some(lamp) => match lamps.iter_mut().find(|(other, _)| *other == lamp) {
                Some((_, list)) => list.extend_from_slice(triangle),
                None => lamps.push((lamp, triangle.to_vec())),
            },
            None => rest.extend_from_slice(triangle),
        }
    }
    (lamps, rest)
}

/// Rev lights fill green, red, then blue and flash on the limiter; during the
/// start sequence they mirror the gantry. Side lamps show grip loss and tow.
pub fn lights(
    session: Res<Session>,
    mut lamps: Query<(&mut WheelLamp, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !session.ready || lamps.is_empty() {
        return;
    }
    let race = &session.race;
    let car = &race.entrants[session.player].car;
    let flash = (race.time * 12.).sin() > 0.;
    let blue = Color::srgb(0.2, 0.45, 1.);
    let amber = Color::srgb(1., 0.62, 0.05);
    let slipping = car
        .wheels
        .iter()
        .filter(|wheel| wheel.locked || wheel.sliding > 0.3)
        .count();
    let tow = car.aero.slipstream().clamp(0., 1.) as f32;
    for (mut state, material) in &mut lamps {
        let lit = match state.lamp {
            Lamp::Rev(index) => hud::wheel::rev_light(&session, index, REV_LAMPS).map(|color| {
                let [r, g, b, _] = color.to_array();
                Color::srgb_u8(r, g, b)
            }),
            Lamp::Left(row) => {
                (car.powertrain.last_refusal.is_some() && flash || slipping > row).then_some(amber)
            }
            Lamp::Right(row) => (tow > (row as f32 + 0.5) / 3.).then_some(blue),
        };
        if lit == state.lit {
            continue;
        }
        state.lit = lit;
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.unlit = lit.is_some();
            material.base_color = lit.unwrap_or(state.off);
        }
    }
}
