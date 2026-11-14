// Whole models that face each view (see `billboard.rs`: `FacesView`): the shield bubble, the bursts,
// a warpball, the hazards whose model points to the camera. The entity is placed and scaled as
// usual, but what its rotation would be is relative to the view: the model is turned by the
// view's own rotation after its scale and its own turn (a child's, or a quarter on the screen), as
// `transform.rotation = camera` was on the CPU. The entity's own rotation is the identity.

import bevy_pbr::render::{
    mesh_functions::get_world_from_local,
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

struct TurnedVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
}

@vertex
fn vertex(in: TurnedVertex) -> ViewOutput {
    let local_to_world = get_world_from_local(in.instance_index);
    let centre = local_to_world[3].xyz;
    let linear = mat3x3<f32>(
        local_to_world[0].xyz,
        local_to_world[1].xyz,
        local_to_world[2].xyz,
    );
    let facing = mat3x3<f32>(
        view.world_from_view[0].xyz,
        view.world_from_view[1].xyz,
        view.world_from_view[2].xyz,
    );
    let world = centre + facing * (linear * in.position);
    return view_output(in.instance_index, world, in.uv, in.color);
}
