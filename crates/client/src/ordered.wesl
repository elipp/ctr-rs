// A model drawn in the depth order of its origin (see `ordered.rs`). Retail links a whole instance
// into the ordering table at one slot, its origin's depth plus its bias, so its triangles are not
// sorted against the level's one by one: a dancer's umbrella is drawn over the pillar it pokes
// into. Here every vertex stays where it is on the screen, and is depth-tested at the origin's
// depth, `nearer` in front of it, keeping `relief` of its own depth from the origin so that the
// model's own faces still sort among themselves.

import bevy_pbr::render::{
    mesh_functions::{get_world_from_local, mesh_position_local_to_world},
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

// `Ordering` of `ordered.rs`.
struct Ordering {
    nearer: f32,
    relief: f32,
    _pad: vec2<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> ordering: Ordering;

struct OrderedVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
}

@vertex
fn vertex(in: OrderedVertex) -> ViewOutput {
    let local_to_world = get_world_from_local(in.instance_index);
    let world = mesh_position_local_to_world(local_to_world, vec4<f32>(in.position, 1.0)).xyz;
    var out = view_output(in.instance_index, world, in.uv, in.color);
    let eye = view.world_from_view[3].xyz;
    let forward = -view.world_from_view[2].xyz;
    let origin = dot(local_to_world[3].xyz - eye, forward);
    let own = dot(world - eye, forward);
    // (Nearer than the camera is at the near plane: retail puts it in the first slot.)
    let depth = max(origin - ordering.nearer + (own - origin) * ordering.relief, 1e-4);
    let clip = view.clip_from_view * vec4<f32>(0.0, 0.0, -depth, 1.0);
    out.position.z = min(clip.z / clip.w, 1.0) * out.position.w;
    return out;
}
