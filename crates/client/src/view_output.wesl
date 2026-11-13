// What the vertex shaders of the effects that face each view (`billboard.rs`) hand to Bevy's
// fragment shader: `bevy_pbr`'s `VertexOutput`, which that reads (what a mesh here does not have is
// left out, as there). A shader gives the world position it ended up with and what its vertex
// carries; the rest is the same for all of them.

import bevy_pbr::render::{
    mesh_view_bindings::view,
    view_transformations::position_world_to_clip,
};

struct ViewOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    @location(6) @interpolate(flat) instance_index: u32,
}

fn view_output(
    instance_index: u32,
    world: vec3<f32>,
    uv: vec2<f32>,
    color: vec4<f32>,
) -> ViewOutput {
    var out: ViewOutput;
    out.world_position = vec4<f32>(world, 1.0);
    out.position = position_world_to_clip(world);
    // Towards the viewer, as the quad is.
    out.world_normal = view.world_from_view[2].xyz;
@if(VERTEX_UVS_A)
    out.uv = uv;
@if(VERTEX_COLORS)
    out.color = color;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = instance_index;
    return out;
}
