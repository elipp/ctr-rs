//! Writing the vertices of a mesh that is redrawn every frame only when they changed.
//!
//! `Assets::get_mut` marks a mesh modified whether or not anything in it changes, and a modified
//! mesh is extracted again: the render world frees its place in the mesh slabs, allocates a new
//! one and uploads all of it (`allocate_and_free_meshes`). Most of the particle, mark and
//! wheel meshes are the same from one frame to the next (empty, or of a kart standing
//! still), so reading them first and leaving them be is far cheaper than writing them.

use bevy::mesh::{Mesh, MeshVertexAttribute, VertexAttributeValues};
use bevy::prelude::*;

/// Sets `attributes` of the mesh `handle`, unless it has every one of them as they are already.
pub(crate) fn set_attributes<const N: usize>(
    meshes: &mut Assets<Mesh>,
    handle: &Handle<Mesh>,
    attributes: [(MeshVertexAttribute, VertexAttributeValues); N],
) {
    let Some(mesh) = meshes.get(handle) else {
        return;
    };
    let same = attributes.iter().all(|(attribute, values)| {
        mesh.try_attribute_option(attribute.id)
            .is_ok_and(|now| now == Some(values))
    });
    if same {
        return;
    }
    let Some(mut mesh) = meshes.get_mut(handle) else {
        return;
    };
    for (attribute, values) in attributes {
        mesh.insert_attribute(attribute, values);
    }
}
