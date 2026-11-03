//! Where the race's 2D art goes. Retail draws it on a 512 by 240 screen, and the HUD here places
//! it the same way, a pixel of that screen being [`PIXEL`] logical pixels. Retail's pixels are
//! taller than wide (`weapon::HUD_PIXEL_ASPECT`), and so are these: at 2 by 3, the 240 lines fill a
//! window 720 pixels tall, and every texel is a whole number of pixels. Like the rest of the HUD,
//! it keeps this size whatever the window's.
//!
//! Every picture of it is an `ImageNode` with `NodeImageMode::Stretch`. The default, `Auto`, draws
//! the picture in its own proportions (square texels) and fits it, centered, into the node, so a
//! node of a retail pixel's tall shape shows it flattened, with gaps above and below. (The game's
//! fonts and the traffic lights were drawn that way for a while.)

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use loader::dram::DramFile;
use loader::icons::{parse_groups, parse_icons};
use loader::source::Need;

/// Retail's screen, in its pixels across and lines down.
pub const WIDTH: i32 = 512;
pub const HEIGHT: i32 = 240;

/// A pixel of retail's screen, in logical pixels.
pub const PIXEL: Vec2 = Vec2::new(2.0, 3.0);

/// `n` pixels of retail's screen across.
pub fn x(n: f32) -> Val {
    Val::Px(n * PIXEL.x)
}

/// `n` lines of retail's screen down.
pub fn y(n: f32) -> Val {
    Val::Px(n * PIXEL.y)
}

/// The icons of a racer's pack, which have the HUD's pictures. Every racer's pack has the same
/// ones; character 0's will do.
pub(crate) fn pack_icons() -> Vec<loader::icons::Icon> {
    parse_icons(&pack()).expect("parse racer pack icons")
}

/// The same pack's icon groups.
pub(crate) fn pack_groups() -> Vec<loader::icons::IconGroup> {
    parse_groups(&pack()).expect("parse racer pack icons")
}

fn pack() -> DramFile {
    let pack = crate::disc::source()
        .get(Need::Pack(0))
        .expect("read racer pack");
    DramFile::parse(&pack).expect("parse racer pack")
}

/// A picture of the HUD as a texture, for the UI to cut from.
pub(crate) fn atlas_image(atlas: render::mesh::Atlas, images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new(
        Extent3d {
            width: atlas.width as u32,
            height: atlas.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        atlas.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    images.add(image)
}
