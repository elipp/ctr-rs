// The wheels (see `wheels.rs`): a square turned to the view that draws it, as retail's
// `DrawTiresSolid` does for its one camera (`render::wheels::wheel_quads` is the same in Rust).
// The entity sits at the wheel's centre. A vertex has which corner of the picture it is (`uv`: 0
// or 1 each way), the axle and the radius (`axle`, the radius 0 when the wheel is hidden) and the
// kart's side the wheel is on and whether it is a right-hand one (`side`).

import bevy_pbr::render::{
    mesh_functions::{get_world_from_local, mesh_position_local_to_world},
    mesh_view_bindings::view,
};
import package::view_output::{ViewOutput, view_output};

// `WheelSprites` of `wheels.rs`: `sDrawTiresSpriteIndexTable` and how many pictures the atlas has.
struct WheelSprites {
    count: u32,
    push: f32,
    table: array<vec4<u32>, 33>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> sprites: WheelSprites;

struct WheelVertex {
    @builtin(instance_index) instance_index: u32,
    @location(2) uv: vec2<f32>,
    @location(8) axle: vec4<f32>,
    @location(9) side: vec4<f32>,
}

fn normalize_or_zero(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if l > 0.0 {
        return v / l;
    }
    return vec3<f32>(0.0);
}

@vertex
fn vertex(in: WheelVertex) -> ViewOutput {
    let local_to_world = get_world_from_local(in.instance_index);
    let center = mesh_position_local_to_world(local_to_world, vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let eye = view.world_from_view[3].xyz;
    let axle = in.axle.xyz;
    let radius = in.axle.w;
    let right_hand = in.side.w > 0.5;

    let to_eye = normalize_or_zero(eye - center);
    let across = normalize_or_zero(cross(to_eye, axle));
    let along = -normalize_or_zero(cross(to_eye, across));
    let angle = dot(axle, to_eye);

    // The corner of the square this vertex is on (p0..p3 are (-1,-1), (1,-1), (-1,1), (1,1)):
    // the picture is mirrored when the axle points away from the view, and the right-hand wheels
    // are the left-hand ones the other way round (`DrawTiresSolid_ApplyCornerOrder`).
    var order = array<u32, 16>(
        3u, 1u, 2u, 0u,
        2u, 0u, 3u, 1u,
        1u, 3u, 0u, 2u,
        0u, 2u, 1u, 3u,
    );
    let corner = u32(in.uv.x + 0.5) + 2u * u32(in.uv.y + 0.5);
    var mode = 0u;
    if right_hand {
        mode += 1u;
    }
    if angle < 0.0 {
        mode += 2u;
    }
    let p = order[mode * 4u + corner];
    let a = f32(p & 1u) * 2.0 - 1.0;
    let b = f32(p >> 1u) * 2.0 - 1.0;
    var world = center + across * (a * radius) + along * (b * radius);

    // A wheel on the view's side of the kart is pushed towards the view, so that the body does not
    // cut into it: moving along the line of sight and shrinking about the eye by the same factor
    // leaves the square looking the same.
    let distance = length(center - eye);
    if dot(in.side.xyz, to_eye) > 0.0 && distance > 0.0 {
        world = eye + (world - eye) * max(1.0 - sprites.push * radius / distance, 0.1);
    }

    // `sDrawTiresSpriteIndexTable[min(|angle| >> 5, 0x80)]`, then the corner's place on its picture.
    let at = min(u32(abs(angle)) >> 5u, 128u);
    let picture = min(sprites.table[at / 4u][at % 4u], sprites.count - 1u);
    let uv = vec2<f32>(in.uv.x, (f32(picture) + in.uv.y) / f32(sprites.count));
    return view_output(in.instance_index, world, uv, vec4<f32>(1.0));
}
