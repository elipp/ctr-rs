//! A kart in mud or water, cut at the water line (`SPLIT_LINE`, `vertSplit` 0). While the sim
//! says it is in it ([`vehicle::Driver::split_line`]) and the model reaches under the line, its
//! body is drawn as two halves of its own: the one over the line as the kart is drawn (over the
//! track, for ours), and the one under it in the world's layer, behind the water surface, which
//! tints it (and a mud surface, which is opaque, hides it). The halves are the kart's
//! triangles split on the line every frame (`render::submerge`), as the retail renderer does.
//!
//! The wake that glows on the line is `projectile`'s, and the bubbles are `exhaust`'s.

use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use render::mesh::{MeshPart, SCALE};
use render::submerge::split;

use crate::kart::{KartRig, KartSlot};
use crate::props::Sim;
use crate::subtract::SubtractMaterial;

/// The water line, in world units (`vehicle::force::WATERLINE_Y`).
const WATERLINE: f32 = vehicle::force::WATERLINE_Y as f32 * SCALE;

/// One part of a kart's body, and the two halves it is drawn as in the water.
struct Twin {
    /// The part's own entity, which is hidden while the halves show.
    whole: Entity,
    mesh: Handle<Mesh>,
    over: Half,
    under: Half,
}

struct Half {
    entity: Entity,
    mesh: Handle<Mesh>,
}

/// The halves a kart's parts are drawn as, made the first time it is in the water.
#[derive(Component, Default)]
pub struct Submerged(Vec<Twin>);

fn part_of(mesh: &Mesh) -> Option<MeshPart> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(v) => v.clone(),
        _ => return None,
    };
    let colors = match mesh.attribute(Mesh::ATTRIBUTE_COLOR)? {
        VertexAttributeValues::Float32x4(v) => v.clone(),
        _ => return None,
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0)? {
        VertexAttributeValues::Float32x2(v) => v.clone(),
        _ => return None,
    };
    let Some(Indices::U32(indices)) = mesh.indices() else {
        return None;
    };
    Some(MeshPart {
        positions,
        colors,
        uvs,
        indices: indices.clone(),
    })
}

fn write(mesh: &mut Mesh, part: MeshPart) {
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, part.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, part.colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, part.uvs);
    mesh.insert_indices(Indices::U32(part.indices));
}

/// Splits the bodies of the karts that are in the water, and shows the halves in place of them.
pub fn cut(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut karts: Query<(
        Entity,
        &Transform,
        &KartSlot,
        &KartRig,
        Option<&Children>,
        Option<&mut Submerged>,
    )>,
    parts: Query<(
        &Mesh3d,
        &RenderLayers,
        Option<&MeshMaterial3d<StandardMaterial>>,
        Option<&MeshMaterial3d<SubtractMaterial>>,
    )>,
    mut visibility: Query<&mut Visibility>,
) {
    for (root, transform, slot, rig, children, submerged) in &mut karts {
        let in_water = sim
            .race
            .karts
            .get(slot.0)
            .is_some_and(|kart| kart.driver.split_line);
        let Some(mut submerged) = submerged else {
            if !in_water {
                continue;
            }
            // The first time: a pair of entities for each part of the body, sharing its
            // materials.
            let mut twins = Vec::new();
            for &child in children.into_iter().flatten() {
                let Ok((mesh, layers, standard, subtract)) = parts.get(child) else {
                    continue;
                };
                if !rig.parts.iter().flatten().any(|part| *part == mesh.0) {
                    continue;
                }
                let Some(source) = meshes.get(&mesh.0).cloned() else {
                    continue;
                };
                let mut half = |layers: RenderLayers| {
                    let handle = meshes.add(source.clone());
                    let mut entity = commands.spawn((
                        Mesh3d(handle.clone()),
                        Transform::default(),
                        layers,
                        Visibility::Hidden,
                        ChildOf(root),
                    ));
                    if let Some(material) = standard {
                        entity.insert(material.clone());
                    }
                    if let Some(material) = subtract {
                        entity.insert(material.clone());
                    }
                    Half {
                        entity: entity.id(),
                        mesh: handle,
                    }
                };
                twins.push(Twin {
                    whole: child,
                    mesh: mesh.0.clone(),
                    over: half(layers.clone()),
                    under: half(RenderLayers::layer(0)),
                });
            }
            commands.entity(root).insert(Submerged(twins));
            continue;
        };

        // Where each part's vertices are over the line, by the pose of the kart this frame.
        let mut cuts = Vec::new();
        let mut reaches_under = false;
        if in_water {
            for twin in &submerged.0 {
                let Some(part) = meshes.get(&twin.mesh).and_then(part_of) else {
                    cuts.push(Default::default());
                    continue;
                };
                let heights: Vec<f32> = part
                    .positions
                    .iter()
                    .map(|&p| transform.transform_point(Vec3::from(p)).y - WATERLINE)
                    .collect();
                reaches_under |= heights.iter().any(|&h| h < 0.0);
                cuts.push(split(&part, &heights));
            }
        }
        let cut = in_water && reaches_under;
        let mut show = |entity: Entity, shown: bool| {
            if let Ok(mut visibility) = visibility.get_mut(entity) {
                visibility.set_if_neq(if shown {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
        };
        let mut cuts = cuts.into_iter();
        for twin in &mut submerged.0 {
            show(twin.whole, !cut);
            let (over, under) = match cut {
                true => cuts.next().unwrap_or_default(),
                false => Default::default(),
            };
            for (half, part) in [(&twin.over, over), (&twin.under, under)] {
                let shown = !part.indices.is_empty();
                show(half.entity, shown);
                if shown && let Some(mut mesh) = meshes.get_mut(&half.mesh) {
                    write(&mut mesh, part);
                }
            }
        }
    }
}
