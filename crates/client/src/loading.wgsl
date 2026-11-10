// The loading screen's checkered flag (`RaceFlag_DrawSelf`, see `loading.rs`). Retail puts a grid
// of gouraud quads through the GTE; here each pixel casts its ray at the same surface instead,
// and finds where it meets it.
//
// The flag is in retail's space: its columns are 100 units apart across (x) and its points 94
// down (y), and z, how deep it is, is the wave of its column plus a ripple. `flag.columns` has
// each column's depth (x) and light (y), the first at column `FIRST`.

const COLUMNS: u32 = 88u;
const FIRST: f32 = -40.0;
const LAST: f32 = 47.0;
// The points of a column the flag has, as the rows of retail's go on above and below its 30.
const TOP: f32 = -32.0;
const BOTTOM: f32 = 70.0;
// How deep the flag can be: 640 plus the wave (up to 182) and the ripple (up to 32).
const NEAR: f32 = 630.0;
const FAR: f32 = 870.0;
const MARCH: i32 = 24;
const BISECT: i32 = 6;

struct Flag {
    columns: array<vec4<f32>, 88>,
    // From the direction of a pixel on retail's screen to the direction in the flag's space.
    unproject: mat4x4<f32>,
    // The flag's first column across (x), the ripple's angle (y), the column of its trailing edge
    // (z), and whether what it misses is covered too (w).
    flag: vec4<f32>,
    // The node's size (xy) and the size of one of retail's pixels in it (zw).
    screen: vec4<f32>,
}

@group(1) @binding(0) var<uniform> flag: Flag;

// Retail's sine: 4096 to a turn, and as much at its peak.
fn sine(a: f32) -> f32 {
    return sin(a * 6.2831853 / 4096.0) * 4096.0;
}

// Where a point of the flag's space is on it: the column across, the point down.
fn grid(p: vec3<f32>) -> vec2<f32> {
    return vec2((p.x - flag.flag.x) / 100.0, (p.y + 910.0) / 94.0);
}

// The column at `c`, between the two it is between, as the gouraud quads shade across.
fn column(c: f32) -> vec4<f32> {
    let i = clamp(c - FIRST, 0.0, f32(COLUMNS - 1u));
    let i0 = u32(floor(i));
    let i1 = min(i0 + 1u, COLUMNS - 1u);
    return mix(flag.columns[i0], flag.columns[i1], i - floor(i));
}

// How deep the flag is under a point of its space.
fn depth(p: vec3<f32>) -> f32 {
    let g = grid(p);
    let ripple = (sine(flag.flag.y + 256.0 * (g.x - FIRST) + 300.0 * g.y) + 4095.0) * 32.0 / 8192.0;
    return column(g.x).x + ripple;
}

// Which square of the checker a coordinate is in (1 or -1), and 0 at its edges, smoothed over a
// pixel.
fn square(x: f32, w: f32) -> f32 {
    let f = fract(x);
    let s = select(-1.0, 1.0, (i32(floor(x)) & 1) == 0);
    return s * clamp(min(f, 1.0 - f) / w * 2.0, 0.0, 1.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return mix(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, step(c, vec3(0.04045)));
}

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // The pixel on retail's screen, from its centre (256, 120).
    let r = (uv * flag.screen.xy - flag.screen.xy * 0.5) / flag.screen.zw;
    let d = (flag.unproject * vec4(r / 256.0, 1.0, 0.0)).xyz;
    let ahead = d.z > 0.0;
    let dz = select(1.0, d.z, ahead);

    // The first step along the ray at which it is past the flag, then closer in on it.
    let s0 = NEAR / dz;
    let s1 = FAR / dz;
    var lo = s0;
    var hi = s1;
    var found = false;
    for (var k = 1; k <= MARCH; k++) {
        if !found {
            let s = s0 + (s1 - s0) * f32(k) / f32(MARCH);
            if s * dz - depth(s * d) >= 0.0 {
                hi = s;
                found = true;
            } else {
                lo = s;
            }
        }
    }
    for (var k = 0; k < BISECT; k++) {
        let s = (lo + hi) * 0.5;
        if s * dz - depth(s * d) >= 0.0 {
            hi = s;
        } else {
            lo = s;
        }
    }
    let g = grid(hi * d);
    let on = ahead && g.x >= flag.flag.z && g.x <= LAST && g.y >= TOP && g.y <= BOTTOM;

    // A quad between columns c - 1 and c is retail's column c; a square is 4 by 4 quads.
    let q = vec2((g.x + 1.0) / 4.0, g.y / 4.0);
    let w = max(fwidth(q), vec2(1e-4));
    let dark = 0.5 - 0.5 * square(q.x, w.x) * square(q.y, w.y);
    let l = column(g.x).y;
    let light_shade = (l * 130.0 + (8192.0 - l) * 255.0) / 8192.0;
    let dark_shade = (l * 105.0 + (8192.0 - l) * 160.0) / 8192.0;
    var shade = mix(light_shade, dark_shade, dark);
    var alpha = 1.0;
    if !on {
        // Past the end of the flag (the corner of a very wide window, which looks past its far
        // end): the grey of the squares from afar while it covers the screen, the scene after.
        shade = 162.5;
        alpha = flag.flag.w;
    }
    return vec4(to_linear(vec3(clamp(shade / 255.0, 0.0, 1.0))), alpha);
}
