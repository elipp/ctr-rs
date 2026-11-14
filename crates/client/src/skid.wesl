// The skid marks' fade, per view (see `skid.rs`). Retail sets how bright a kart's marks are from
// the distance of its newest mark to the camera; here that is the vertex shader's job, so every
// view of a split screen fades them by its own camera. A vertex has the kart's fade anchor
// (`anchor`, in world units), how many times its brightness is halved for its age (`color.x`) and
// the gain of the kind of mark (`color.y`).

import bevy_pbr::render::{
    mesh_functions::{get_world_from_local, mesh_position_local_to_world},
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

// `SkidFade` of `skid.rs`: the table has the brightness for the full-intensity depth (at 0) and for
// each count of bits of the depth past it (at 1 and up).
struct SkidFade {
    scale: f32,
    cull: f32,
    near: i32,
    table: array<vec4<u32>, 9>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> fade: SkidFade;

struct SkidVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
    @location(8) anchor: vec3<f32>,
}

// How bright the newest marks are (0..=0x7f) for this view: `vehicle::skid::intensity` of the
// anchor's depth in front of the camera, and 0 when it is too far on any axis.
fn brightness(anchor: vec3<f32>) -> u32 {
    let eye = view.world_from_view[3].xyz;
    let forward = -view.world_from_view[2].xyz;
    let from_eye = (anchor - eye) / fade.scale;
    let farthest = max(abs(from_eye.x), max(abs(from_eye.y), abs(from_eye.z)));
    if farthest >= fade.cull {
        return 0u;
    }
    let depth = i32(dot(from_eye, forward));
    var slot = 0u;
    if depth >= fade.near {
        slot = 33u - countLeadingZeros(u32(depth - fade.near));
    }
    return fade.table[slot / 4u][slot % 4u];
}

@vertex
fn vertex(in: SkidVertex) -> ViewOutput {
    let local_to_world = get_world_from_local(in.instance_index);
    let world = mesh_position_local_to_world(local_to_world, vec4<f32>(in.position, 1.0));
    let anchor = mesh_position_local_to_world(local_to_world, vec4<f32>(in.anchor, 1.0));
    // The primitive's colour scales the texture.
    let value = brightness(anchor.xyz) >> u32(in.color.x);
    let gain = f32(value) / 128.0 * in.color.y;
    return view_output(
        in.instance_index,
        world.xyz,
        in.uv,
        vec4<f32>(gain, gain, gain, 1.0),
    );
}
