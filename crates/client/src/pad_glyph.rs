//! PlayStation button pictures for a `{cross}` and the like in a [`CtrText`](crate::font::CtrText)
//! (`font::lay_out`), and a keycap for a `{key:H}`. Cut from Max_the_Viking's sheet and
//! `keycap.png` by `assets/ps4-atlas.py`.

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::sprite::{BorderRect, SliceScaleMode, TextureSlicer};

pub const ATLAS: Handle<Image> = bevy::asset::uuid_handle!("fb87966f-130a-4350-aa68-a65fee6e60bd");

const CELL: u32 = 128;
const COLUMNS: u32 = 4;
/// Down to 8-pixel cells: drawn a few times smaller than a cell, unmipped the outlines shimmer.
const MIP_LEVELS: u32 = 5;

/// In the atlas's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadGlyph {
    Select,
    Start,
    Triangle,
    Square,
    Circle,
    Cross,
    L,
    DpadDown,
    DpadLeft,
    DpadRight,
    DpadUp,
    R,
    L3,
    R3,
}

impl PadGlyph {
    const ALL: [PadGlyph; 14] = [
        PadGlyph::Select,
        PadGlyph::Start,
        PadGlyph::Triangle,
        PadGlyph::Square,
        PadGlyph::Circle,
        PadGlyph::Cross,
        PadGlyph::L,
        PadGlyph::DpadDown,
        PadGlyph::DpadLeft,
        PadGlyph::DpadRight,
        PadGlyph::DpadUp,
        PadGlyph::R,
        PadGlyph::L3,
        PadGlyph::R3,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PadGlyph::Select => "select",
            PadGlyph::Start => "start",
            PadGlyph::Triangle => "triangle",
            PadGlyph::Square => "square",
            PadGlyph::Circle => "circle",
            PadGlyph::Cross => "cross",
            PadGlyph::L => "l",
            PadGlyph::DpadDown => "dpad_down",
            PadGlyph::DpadLeft => "dpad_left",
            PadGlyph::DpadRight => "dpad_right",
            PadGlyph::DpadUp => "dpad_up",
            PadGlyph::R => "r",
            PadGlyph::L3 => "l3",
            PadGlyph::R3 => "r3",
        }
    }

    /// Any case: a text is often uppercased whole after its `{cross}` went in.
    pub fn parse(name: &str) -> Option<PadGlyph> {
        PadGlyph::ALL
            .into_iter()
            .find(|g| g.name().eq_ignore_ascii_case(name))
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Its cell in [`ATLAS`], in the top level's pixels.
    pub fn rect(self) -> Rect {
        cell(self.index() as u32)
    }
}

fn cell(i: u32) -> Rect {
    let min = UVec2::new(i % COLUMNS, i / COLUMNS) * CELL;
    Rect::from_corners(min.as_vec2(), (min + CELL).as_vec2())
}

/// The keycap's cell in [`ATLAS`]: the one after the PS glyphs.
pub fn keycap_rect() -> Rect {
    cell(PadGlyph::ALL.len() as u32)
}

/// The keycap is drawn as tall as a PS glyph, and only ever wider: columns 33..94 of its cell
/// are all alike, which stretch. Its corners then scale with its height (`max_corner_scale`
/// caps them at its height's scale), so the rounding keeps its shape.
pub fn keycap_slicer() -> TextureSlicer {
    TextureSlicer {
        border: BorderRect {
            min_inset: Vec2::new(33.0, 0.0),
            max_inset: Vec2::new(34.0, 0.0),
        },
        center_scale_mode: SliceScaleMode::Stretch,
        sides_scale_mode: SliceScaleMode::Stretch,
        max_corner_scale: 1.0,
    }
}

/// The keycap's top face, which its letters go on, as fractions of its cell (x, then y).
pub const KEYCAP_FACE: Rect = Rect {
    min: Vec2::new(15.0 / 128.0, 3.0 / 128.0),
    max: Vec2::new(113.0 / 128.0, 100.0 / 128.0),
};

/// Puts [`ATLAS`] in the image assets, mipmapped.
pub fn register(images: &mut Assets<Image>) {
    let png = Image::from_buffer(
        include_bytes!("../images/pad-glyphs-128.png"),
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::default(),
    )
    .expect("the pad glyphs are a PNG")
    .try_into_dynamic()
    .expect("the pad glyphs are RGBA")
    .into_rgba8();
    let (width, height) = png.dimensions();
    assert_eq!(width % (CELL >> (MIP_LEVELS - 1)), 0);
    let top = png.into_raw();
    let mut mips = Vec::new();
    let mut level = top.clone();
    for l in 1..MIP_LEVELS {
        level = halve(&level, (width >> (l - 1)) as usize, (height >> (l - 1)) as usize);
        mips.extend_from_slice(&level);
    }
    // `Image::new` wants the top level alone.
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        top,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data.as_mut().expect("just given").extend(mips);
    image.texture_descriptor.mip_level_count = MIP_LEVELS;
    image.sampler = ImageSampler::linear();
    images
        .insert(ATLAS.id(), image)
        .expect("the pad glyphs' handle is free");
}

/// Each 2x2 block of an RGBA8 image averaged, colour weighted by alpha so that the transparent
/// texels' (black) colour doesn't darken the edges. Cells are a power of two and aligned, so no
/// block spans two of them.
fn halve(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let (w, h) = (width / 2, height / 2);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = &rgba[((2 * y + dy) * width + 2 * x + dx) * 4..][..4];
                let a = p[3] as u32;
                for c in 0..3 {
                    sum[c] += p[c] as u32 * a;
                }
                sum[3] += a;
            }
            let o = &mut out[(y * w + x) * 4..][..4];
            if sum[3] > 0 {
                for c in 0..3 {
                    o[c] = ((sum[c] + sum[3] / 2) / sum[3]) as u8;
                }
            }
            o[3] = ((sum[3] + 2) / 4) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_back() {
        for g in PadGlyph::ALL {
            assert_eq!(PadGlyph::parse(&g.name().to_uppercase()), Some(g));
        }
        assert_eq!(PadGlyph::parse("nope"), None);
    }

    #[test]
    fn registers() {
        let mut images = Assets::<Image>::default();
        register(&mut images);
        let image = images.get(&ATLAS).unwrap();
        assert_eq!(image.size(), UVec2::splat(CELL * COLUMNS));
        // The 14 glyphs and the keycap fit the 4x4 grid.
        assert!(keycap_rect().max.y <= (CELL * COLUMNS) as f32);
    }

    #[test]
    fn halving_ignores_transparent_colour() {
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(halve(&rgba, 2, 2), [255, 0, 0, 64]);
    }
}
