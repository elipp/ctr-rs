//! Models drawn in the depth order of their origin, as retail orders an instance: one slot of the
//! ordering table for all of it, at its origin's depth (`viewDepth >> 6`, a slot to 64 units)
//! plus its depth bias (`RenderBucket_QueueDraw`). Its faces are not sorted against the level's
//! one by one, so a part of it that pokes into a wall is drawn over the wall. `ordered.wesl` puts
//! the depth of each vertex at the origin's, nearer by the bias.
//!
//! [`OrderedAsOne`] on the root of a model made by `spawn_parts` gives its parts the material.

use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use render::mesh::SCALE;

use crate::billboard::view_extension;

/// The game units of depth in one slot of the ordering table.
const SLOT: f32 = 64.0;

/// How much of a model's own depth from its origin is kept, for its faces to sort among
/// themselves (retail sorts them in the model's own part of the table).
const RELIEF: f32 = 0.125;

/// What `ordered.wesl` needs: how far nearer than its origin the model is drawn, and how much of
/// its own depth it keeps (world units).
#[derive(ShaderType, Clone, Debug, Default)]
pub struct Ordering {
    nearer: f32,
    relief: f32,
}

view_extension!(
    /// A model depth-tested at its origin's depth (`ordered.wesl`).
    Ordered => OrderedMaterial,
    shader: "ordered.wesl",
    subtract: false,
    attributes: [
        Mesh::ATTRIBUTE_POSITION => 0,
        Mesh::ATTRIBUTE_UV_0 => 2,
        Mesh::ATTRIBUTE_COLOR => 5,
    ],
    uniform: ordering: Ordering,
);

/// On the root of a model: its parts are drawn in its origin's depth order, `bias` slots of the
/// ordering table farther (negative: nearer), as `Instance::depthBiasNormal` has it. The parts must
/// be its children.
#[derive(Component, Clone, Copy)]
pub struct OrderedAsOne {
    pub bias: i32,
}

/// Gives the parts of an [`OrderedAsOne`] model the material, the frame they appear. (Its parts
/// blended as `subtract.rs` does keep theirs.)
fn ordered(
    mut commands: Commands,
    roots: Query<&OrderedAsOne>,
    parts: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, &ChildOf),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<OrderedMaterial>>,
) {
    for (entity, handle, parent) in &parts {
        let Ok(root) = roots.get(parent.parent()) else {
            continue;
        };
        let Some(base) = standard.get(&handle.0).cloned() else {
            continue;
        };
        let ordering = Ordering {
            nearer: -(root.bias as f32) * SLOT * SCALE,
            relief: RELIEF,
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(materials.add(OrderedMaterial {
                base,
                extension: Ordered { ordering },
            })));
    }
}

pub struct OrderedPlugin;

impl Plugin for OrderedPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "ordered.wesl");
        app.add_plugins(MaterialPlugin::<OrderedMaterial>::default())
            .add_systems(
                PostUpdate,
                ordered.before(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}
