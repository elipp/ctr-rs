// The speedometer's needle: a kite of two gouraud halves, with its edges smoothed.

struct Needle {
    tip: vec2<f32>,
    tail: vec2<f32>,
    bright_side: vec2<f32>,
    dark_side: vec2<f32>,
}

@group(1) @binding(0) var<uniform> needle: Needle;

// Positive to the inside, going round the kite from the tip through its bright side.
fn edge(p: vec2<f32>, q: vec2<f32>, x: vec2<f32>) -> f32 {
    let d = q - p;
    return d.x * (x.y - p.y) - d.y * (x.x - p.x);
}

fn coverage(e: f32) -> f32 {
    return clamp(e / length(vec2(dpdx(e), dpdy(e))) + 0.5, 0.0, 1.0);
}

// The colours are retail's, interpolated in sRGB as the PS1 does.
fn shade(a: vec2<f32>, b: vec2<f32>, c: vec2<f32>, x: vec2<f32>, ca: vec3<f32>, cb: vec3<f32>, cc: vec3<f32>) -> vec3<f32> {
    return (edge(b, c, x) * ca + edge(c, a, x) * cb + edge(a, b, x) * cc) / edge(a, b, c);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return mix(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, step(c, vec3(0.04045)));
}

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let tip = needle.tip;
    let tail = needle.tail;
    let bright = needle.bright_side;
    let dark = needle.dark_side;

    let alpha = min(
        min(coverage(edge(tip, bright, uv)), coverage(edge(bright, tail, uv))),
        min(coverage(edge(tail, dark, uv)), coverage(edge(dark, tip, uv))),
    );
    let bright_color = shade(tip, tail, bright, uv,
        vec3(255.0, 255.0, 255.0) / 255.0, vec3(156.0, 105.0, 0.0) / 255.0, vec3(255.0, 255.0, 0.0) / 255.0);
    let dark_color = shade(tip, tail, dark, uv,
        vec3(91.0, 91.0, 0.0) / 255.0, vec3(50.0, 43.0, 1.0) / 255.0, vec3(255.0, 187.0, 0.0) / 255.0);
    let color = mix(bright_color, dark_color, coverage(edge(tip, tail, uv)));
    return vec4(to_linear(clamp(color, vec3(0.0), vec3(1.0))), alpha);
}
