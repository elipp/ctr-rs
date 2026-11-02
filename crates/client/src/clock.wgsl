// The clock's trail and flash over the picture (see `clock.rs`).

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;
@group(0) @binding(2) var history: texture_2d<f32>;

struct ClockTrail {
    mode: u32,
    keep: f32,
    inset: vec2<f32>,
}
@group(0) @binding(3) var<uniform> trail: ClockTrail;

const FLASH: u32 = 3u;

struct Input {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// The picture is stored as sRGB and read as linear; the PS1 blends what is stored.
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fragment(in: Input) -> @location(0) vec4<f32> {
    if trail.mode == FLASH {
        return vec4(1.0);
    }
    let now = textureSample(screen_texture, texture_sampler, in.uv).rgb;
    // The last picture, a little in from each edge, over the whole view: it zooms outward.
    let source = trail.inset + in.uv * (1.0 - 2.0 * trail.inset);
    let before = textureSample(history, texture_sampler, source).rgb;
    let blended = mix(to_srgb(now), to_srgb(before), trail.keep);
    return vec4(to_linear(blended), 1.0);
}
