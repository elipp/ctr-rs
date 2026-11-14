// Camera-facing quads (see `billboard.rs`). A vertex is the centre of its sprite (`position`) and
// the offset from it in the view's plane (`corner`, in world units, already turned by the
// particle's rotation). The corner is put on the view's right and up as the view that is drawing
// has them, so every camera and viewport gets a quad that faces it.

import bevy_pbr::render::{
    mesh_functions::{get_world_from_local, mesh_position_local_to_world},
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

struct BillboardVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
    @location(8) corner: vec2<f32>,
}

@vertex
fn vertex(in: BillboardVertex) -> ViewOutput {
    let right = view.world_from_view[0].xyz;
    let up = view.world_from_view[1].xyz;
    let centre = mesh_position_local_to_world(
        get_world_from_local(in.instance_index),
        vec4<f32>(in.position, 1.0),
    );
    let world = centre.xyz + right * in.corner.x + up * in.corner.y;
    return view_output(in.instance_index, world, in.uv, in.color);
}
