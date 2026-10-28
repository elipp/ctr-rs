// The minimap's picture, premultiplied (see `minimap.rs`): drawn as it is, with a blend that
// adds the texels of alpha 0 to what is behind them.

@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(picture, picture_sampler, uv);
}
