// The heat shimmer of the flame jets (see `heat.rs`): lenses that warp the picture, drawn as
// `Torch_Main` does. Each lens is three octagons round a centre (the radii of its rings), and the
// triangles between them show the picture from other places than they cover.

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var texture_sampler: sampler;

const LENSES: u32 = 12u;
// Per lens: the box round it (min x, min y, max x, max y), its centre, and the 8 points of each of
// its 3 rings: top, top right, right, bottom right, bottom, bottom left, left, top left.
const PER_LENS: u32 = 26u;

struct HeatWarp {
    count: u32,
    points: array<vec4<f32>, 312>,
}
@group(0) @binding(2) var<uniform> heat: HeatWarp;

struct Input {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

fn point(lens: u32, ring: u32, i: u32) -> vec2<f32> {
    return heat.points[lens * PER_LENS + 2u + ring * 8u + i].xy;
}

fn centre(lens: u32) -> vec2<f32> {
    return heat.points[lens * PER_LENS + 1u].xy;
}

// Where in the triangle (a, b, c) the point p is, if it is in it: its barycentric coordinates,
// and whether it is.
fn barycentric(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> vec4<f32> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d = v0.x * v1.y - v1.x * v0.y;
    if abs(d) < 1e-12 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
    let v = (v2.x * v1.y - v1.x * v2.y) / d;
    let w = (v0.x * v2.y - v2.x * v0.y) / d;
    let u = 1.0 - v - w;
    let inside = select(0.0, 1.0, u >= 0.0 && v >= 0.0 && w >= 0.0);
    return vec4<f32>(u, v, w, inside);
}

// The place the picture is taken from for the pixel p, where the triangle (da, db, dc) is drawn
// showing the picture over (sa, sb, sc); p itself where it is not in it.
fn warp_tri(
    p: vec2<f32>,
    da: vec2<f32>, db: vec2<f32>, dc: vec2<f32>,
    sa: vec2<f32>, sb: vec2<f32>, sc: vec2<f32>,
    otherwise: vec2<f32>,
) -> vec2<f32> {
    let b = barycentric(p, da, db, dc);
    if b.w > 0.5 {
        return sa * b.x + sb * b.y + sc * b.z;
    }
    return otherwise;
}

// One sector of a lens, between the ring points a and b. `draw_a` and `draw_b` are the rings the
// triangle round the centre reaches at each, `from_a` and `from_b` the rings it shows the picture
// of; the band out to the outermost ring does the same. `outer_first` is the order a quad's
// corners are given in (it decides the diagonal it is cut by).
fn warp_sector(
    p: vec2<f32>, lens: u32, a: u32, b: u32,
    draw_a: u32, draw_b: u32, from_a: u32, from_b: u32, outer_first: bool,
    at: vec2<f32>,
) -> vec2<f32> {
    var to = at;
    let c = centre(lens);
    to = warp_tri(p, c, point(lens, draw_a, a), point(lens, draw_b, b),
                  c, point(lens, from_a, a), point(lens, from_b, b), to);
    // The quad's corners, to draw and to take from, in the order it is cut in.
    var dv: array<vec2<f32>, 4>;
    var sv: array<vec2<f32>, 4>;
    if outer_first {
        dv = array<vec2<f32>, 4>(point(lens, 0u, a), point(lens, 0u, b),
                                 point(lens, draw_a, a), point(lens, draw_b, b));
        sv = array<vec2<f32>, 4>(point(lens, 0u, a), point(lens, 0u, b),
                                 point(lens, from_a, a), point(lens, from_b, b));
    } else {
        dv = array<vec2<f32>, 4>(point(lens, draw_a, a), point(lens, draw_b, b),
                                 point(lens, 0u, a), point(lens, 0u, b));
        sv = array<vec2<f32>, 4>(point(lens, from_a, a), point(lens, from_b, b),
                                 point(lens, 0u, a), point(lens, 0u, b));
    }
    to = warp_tri(p, dv[0], dv[1], dv[2], sv[0], sv[1], sv[2], to);
    to = warp_tri(p, dv[1], dv[2], dv[3], sv[1], sv[2], sv[3], to);
    return to;
}

// Where a lens takes the picture at p from.
fn warp_lens(p: vec2<f32>, lens: u32) -> vec2<f32> {
    let bounds = heat.points[lens * PER_LENS];
    if p.x < bounds.x || p.y < bounds.y || p.x > bounds.z || p.y > bounds.w {
        return p;
    }
    var q = p;
    // The right half squeezes the middle (the picture of ring 1 over ring 2), the left half
    // spreads it, and the two sectors at the bottom left and the top left twist.
    q = warp_sector(p, lens, 1u, 0u, 2u, 2u, 1u, 1u, false, q);
    q = warp_sector(p, lens, 1u, 2u, 2u, 2u, 1u, 1u, false, q);
    q = warp_sector(p, lens, 2u, 3u, 2u, 2u, 1u, 1u, false, q);
    q = warp_sector(p, lens, 3u, 4u, 2u, 2u, 1u, 1u, false, q);
    q = warp_sector(p, lens, 5u, 4u, 1u, 2u, 2u, 1u, true, q);
    q = warp_sector(p, lens, 6u, 5u, 1u, 1u, 2u, 2u, true, q);
    q = warp_sector(p, lens, 7u, 6u, 1u, 1u, 2u, 2u, true, q);
    q = warp_sector(p, lens, 7u, 0u, 1u, 2u, 2u, 1u, true, q);
    return q;
}

@fragment
fn fragment(in: Input) -> @location(0) vec4<f32> {
    // The lenses are in the order they are drawn in, the farthest first; each shows the picture
    // as the ones before it left it, so the last one is looked through first.
    var uv = in.uv;
    for (var i = 0u; i < LENSES; i++) {
        if i >= heat.count {
            break;
        }
        let lens = heat.count - 1u - i;
        uv = warp_lens(uv, lens);
    }
    return textureSample(screen_texture, texture_sampler, uv);
}
