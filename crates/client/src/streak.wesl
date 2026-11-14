// The sparks' streaks (see `billboard.rs`). A streak is a strip from a head to a tail that is
// turned about its own axis to face the view that draws it. A vertex is on the head (`position`)
// and carries the whole way to the tail (`along`), which end it is (`streak.y`: 0 the head, 1 the
// tail) and how far to its side it goes (`streak.x`, in world units, negative on the other side).
// The side is across the axis and the line to the view's eye.

import bevy_pbr::render::{
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

struct StreakVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(5) color: vec4<f32>,
    @location(8) streak: vec2<f32>,
    @location(9) along: vec3<f32>,
}

@vertex
fn vertex(in: StreakVertex) -> ViewOutput {
    let world_from_local = get_world_from_local(in.instance_index);
    let head = (world_from_local * vec4<f32>(in.position, 1.0)).xyz;
    let along = (world_from_local * vec4<f32>(in.along, 0.0)).xyz;
    let eye = view.world_from_view[3].xyz;
    let across = cross(along, eye - head);
    let length_across = length(across);
    var side = vec3<f32>(0.0);
    if length_across > 1e-12 {
        side = across / length_across;
    }
    let world = head + along * in.streak.y + side * in.streak.x;
    return view_output(in.instance_index, world, vec2<f32>(0.0), in.color);
}
