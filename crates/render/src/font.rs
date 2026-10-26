//! The game's two fonts, `largefont` and `middlefont` (icon groups 4 and 5 of a racer's pack):
//! which glyph a character is, where each glyph goes on a line, and the glyphs as a picture.
//!
//! The glyphs are grey. The game tints each one with a colour at its top and another at its
//! bottom (a Gouraud-shaded sprite: the texel times the colour, 0x80 being the texel as it is),
//! which is the orange-to-red of the menus. A [`Gradient`] is those two colours, and
//! [`FontAtlas::tinted`] bakes one into a picture of the glyphs, so that a UI can draw a glyph
//! as a plain image.
//!
//! Not checked against the reference: how far apart the glyphs are (here each glyph takes the
//! columns it has texels in, and a gap of one texel; retail has its own widths), and the colours
//! of the gradients, which are taken from screenshots.

use loader::icons::{Icon, IconGroup};
use loader::vram::Vram;

use crate::mesh::Atlas;

/// One of the game's fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    /// 16x16 glyphs: the menus' titles and items, the big numbers of the HUD.
    Large,
    /// 12x8 glyphs: the smaller text.
    Middle,
}

impl Font {
    /// `IconGroup::group_id` of the font in a racer's pack.
    pub const fn group_id(self) -> i16 {
        match self {
            Font::Large => 4,
            Font::Middle => 5,
        }
    }

    /// The height of a line, in texels: the height of a letter.
    pub const fn line_height(self) -> usize {
        match self {
            Font::Large => 16,
            Font::Middle => 8,
        }
    }

    /// How far a space moves the next glyph, in texels.
    const fn space(self) -> usize {
        match self {
            Font::Large => 8,
            Font::Middle => 6,
        }
    }
}

/// The texels between two glyphs on a line.
const GAP: usize = 1;

/// The glyph of `?`, which stands in for a character the fonts do not have.
const QUESTION: usize = 46;

/// The glyph of a character: its index in the font's icon group, the same in both fonts. Letters
/// are capitals, whatever their case. A space, and anything else the fonts do not have, is none.
pub fn glyph(c: char) -> Option<usize> {
    let c = c.to_ascii_uppercase();
    Some(match c {
        '0'..='9' => c as usize - '0' as usize,
        'A'..='Z' => 10 + c as usize - 'A' as usize,
        '\'' => 36,
        '!' => 37,
        ':' => 38,
        ',' => 39,
        '=' => 41,
        '-' => 42,
        '%' => 43,
        '.' => 44,
        '+' => 45,
        '?' => QUESTION,
        '_' => 47,
        '/' => 48,
        _ => return None,
    })
}

/// The colours a glyph is tinted with at its top and at its bottom, as the GPU's vertex colours:
/// 0x80 keeps the texel as it is, and more brightens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Gradient {
    pub top: [u8; 3],
    pub bottom: [u8; 3],
}

impl Gradient {
    /// The menus' orange: yellow at the top, red at the bottom.
    pub const ORANGE: Gradient = Gradient {
        top: [0xff, 0xc8, 0x30],
        bottom: [0xe0, 0x30, 0x10],
    };

    /// The white of the item a menu has picked.
    pub const WHITE: Gradient = Gradient {
        top: [0xff, 0xff, 0xff],
        bottom: [0xb0, 0xb0, 0xb0],
    };

    /// Redder than the orange: the suffix of a place, the lap of a lap time.
    pub const RED: Gradient = Gradient {
        top: [0xff, 0x70, 0x20],
        bottom: [0xc8, 0x18, 0x10],
    };

    /// The colour at `row` of a glyph `height` texels tall.
    fn at(&self, row: usize, height: usize) -> [u32; 3] {
        let (row, height) = (row as u32, height as u32);
        std::array::from_fn(|i| {
            let (top, bottom) = (self.top[i] as u32, self.bottom[i] as u32);
            (top * (height - row) + bottom * row) / height
        })
    }
}

/// Where a glyph is in its font's atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    /// The left edge of its cell.
    pub x: usize,
    pub width: usize,
    pub height: usize,
    /// The columns of the cell that have texels, `start..end`: what a line shows of it.
    pub ink: (usize, usize),
}

impl Glyph {
    /// How wide it is on a line.
    pub fn ink_width(&self) -> usize {
        self.ink.1 - self.ink.0
    }
}

/// A glyph on a line, and where its first column of texels goes, from the line's left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub glyph: Glyph,
    pub x: usize,
}

/// A line of text in texels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub glyphs: Vec<Placed>,
    pub width: usize,
    pub height: usize,
}

/// The glyphs of a font, side by side in their grey, untinted texels.
pub struct FontAtlas {
    font: Font,
    glyphs: Vec<Glyph>,
    /// How wide every digit is on a line: the widest one's width.
    digit_width: usize,
    /// The grey picture of the glyphs: one row of cells, as tall as the tallest.
    atlas: Atlas,
}

impl FontAtlas {
    /// The font's glyphs, out of a racer pack's icon groups and the texture pages they are cut
    /// from (`shared.vrm`). None if the pack does not have the font.
    pub fn new(font: Font, groups: &[IconGroup], vram: &Vram) -> Option<FontAtlas> {
        let icons = &groups.iter().find(|g| g.group_id == font.group_id())?.icons;
        let size = |icon: &Icon| {
            let [(u0, v0), (u1, _), (_, v2), _] = icon.layout.uv;
            (
                u1.wrapping_sub(u0) as usize + 1,
                v2.wrapping_sub(v0) as usize + 1,
            )
        };
        let width = icons.iter().map(|i| size(i).0).sum();
        let height = icons.iter().map(|i| size(i).1).max()?;
        let mut rgba = vec![0u8; width * height * 4];
        let mut glyphs = Vec::with_capacity(icons.len());
        let mut x = 0;
        for icon in icons {
            let (w, h) = size(icon);
            let (u0, v0) = icon.layout.uv[0];
            let mut ink = (w, 0);
            for v in 0..h {
                for u in 0..w {
                    let texel = vram.texel_stp(
                        &icon.layout,
                        u0.wrapping_add(u as u8),
                        v0.wrapping_add(v as u8),
                    );
                    let Ok(Some((c, _))) = texel else {
                        continue;
                    };
                    rgba[(v * width + x + u) * 4..][..4].copy_from_slice(&c);
                    ink = (ink.0.min(u), ink.1.max(u + 1));
                }
            }
            glyphs.push(Glyph {
                x,
                width: w,
                height: h,
                ink: if ink.0 < ink.1 { ink } else { (0, w) },
            });
            x += w;
        }
        Some(FontAtlas {
            font,
            digit_width: glyphs.iter().take(10).map(Glyph::ink_width).max()?,
            glyphs,
            atlas: Atlas {
                width,
                height,
                rgba,
            },
        })
    }

    pub fn font(&self) -> Font {
        self.font
    }

    pub fn glyphs(&self) -> &[Glyph] {
        &self.glyphs
    }

    /// Where the glyphs of `text` go. A character the font does not have shows as `?`. Every digit
    /// is as wide as the widest, in the middle of that width, so that a number that counts does
    /// not change its width.
    pub fn layout(&self, text: &str) -> Line {
        let mut glyphs = Vec::new();
        let (mut x, mut width) = (0, 0);
        for c in text.chars() {
            if c == ' ' {
                x += self.font.space();
                continue;
            }
            let Some(&glyph) = self.glyphs.get(glyph(c).unwrap_or(QUESTION)) else {
                continue;
            };
            let advance = if c.is_ascii_digit() {
                self.digit_width
            } else {
                glyph.ink_width()
            };
            glyphs.push(Placed {
                glyph,
                x: x + (advance - glyph.ink_width()) / 2,
            });
            x += advance;
            width = x;
            x += GAP;
        }
        Line {
            glyphs,
            width,
            height: self.font.line_height(),
        }
    }

    /// The picture of the glyphs, each tinted from the gradient's top colour at its top row to
    /// the bottom colour at its bottom, as the PS1 multiplies a texel by a vertex colour.
    pub fn tinted(&self, gradient: Gradient) -> Atlas {
        let mut atlas = Atlas {
            width: self.atlas.width,
            height: self.atlas.height,
            rgba: self.atlas.rgba.clone(),
        };
        for glyph in &self.glyphs {
            for y in 0..glyph.height {
                let colour = gradient.at(y, glyph.height);
                for x in glyph.x..glyph.x + glyph.width {
                    let texel = &mut atlas.rgba[(y * atlas.width + x) * 4..][..3];
                    for (t, c) in texel.iter_mut().zip(colour) {
                        *t = (*t as u32 * c / 0x80).min(0xff) as u8;
                    }
                }
            }
        }
        atlas
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::BigIndex;
    use loader::disc::open_bigfile;
    use loader::dram::DramFile;
    use loader::icons::parse_groups;
    use loader::vram::parse_vram_file;

    #[test]
    fn characters_map_to_their_glyphs() {
        assert_eq!(glyph('0'), Some(0));
        assert_eq!(glyph('9'), Some(9));
        assert_eq!(glyph('A'), Some(10));
        assert_eq!(glyph('z'), Some(35));
        assert_eq!(glyph('/'), Some(48));
        assert_eq!(glyph(' '), None);
        assert_eq!(glyph('ä'), None);
    }

    #[test]
    fn gradients_go_from_top_to_bottom() {
        let g = Gradient::ORANGE;
        assert_eq!(g.at(0, 16), g.top.map(u32::from));
        assert_eq!(g.at(8, 16), [0xef, 0x7c, 0x20]);
    }

    /// Both fonts from character 0's pack, or none without the disc image.
    fn fonts() -> Option<[FontAtlas; 2]> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let (mut disc, big) = open_bigfile(&path).ok()?;
        let mut vram = Vram::default();
        let shared = big.read(&mut disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
        for img in parse_vram_file(&shared).unwrap() {
            vram.upload(&img).unwrap();
        }
        let pack = big.read(&mut disc, BigIndex::ARCADE_1P_PACK.get()).unwrap();
        let groups = parse_groups(&DramFile::parse(&pack).unwrap()).unwrap();
        Some([Font::Large, Font::Middle].map(|f| FontAtlas::new(f, &groups, &vram).unwrap()))
    }

    #[test]
    fn fonts_decode_from_the_disc() {
        let Some(fonts) = fonts() else {
            return;
        };
        for atlas in &fonts {
            assert_eq!(atlas.glyphs().len(), 57, "{:?}", atlas.font());
            // Every digit and letter has texels, and is no wider than its cell.
            for glyph in &atlas.glyphs()[..36] {
                assert!(glyph.ink_width() > 0 && glyph.ink.1 <= glyph.width);
                assert_eq!(glyph.height, atlas.font().line_height());
            }
        }
    }

    #[test]
    fn a_line_is_its_glyphs_one_texel_apart() {
        let Some([large, _]) = fonts() else {
            return;
        };
        let line = large.layout("Paused?");
        let ink = |c| large.glyphs()[glyph(c).unwrap()].ink_width();
        assert_eq!(line.glyphs.len(), 7);
        assert_eq!(line.glyphs[1].x, ink('P') + GAP);
        let letters: usize = "PAUSED?".chars().map(ink).sum();
        assert_eq!(line.width, letters + 6 * GAP);
        // A space is wider than a gap, and an unknown character is a question mark.
        assert_eq!(
            large.layout("A A").width,
            2 * ink('A') + Font::Large.space() + GAP
        );
        assert_eq!(large.layout("ä").glyphs, large.layout("?").glyphs);
    }

    #[test]
    fn digits_are_all_as_wide() {
        let Some(fonts) = fonts() else {
            return;
        };
        for atlas in &fonts {
            let one = atlas.layout("1");
            assert_eq!(one.width, atlas.layout("4").width);
            assert_eq!(atlas.layout("10").width, 2 * one.width + GAP);
            // The narrow 1 is in the middle of its width.
            let glyph = one.glyphs[0];
            assert_eq!(
                glyph.x * 2 + glyph.glyph.ink_width(),
                one.width - (one.width - glyph.glyph.ink_width()) % 2
            );
        }
    }

    #[test]
    fn orange_is_yellower_at_the_top() {
        let Some([large, _]) = fonts() else {
            return;
        };
        let atlas = large.tinted(Gradient::ORANGE);
        let a = large.glyphs()[glyph('A').unwrap()];
        // The mean of green over red in each row with texels: the top is yellow, the bottom red.
        let greens: Vec<f32> = (0..a.height)
            .filter_map(|y| {
                let row: Vec<_> = (a.x..a.x + a.width)
                    .map(|x| &atlas.rgba[(y * atlas.width + x) * 4..][..4])
                    .filter(|p| p[3] != 0 && p[0] > 0)
                    .collect();
                let sum: f32 = row.iter().map(|p| p[1] as f32 / p[0] as f32).sum();
                (!row.is_empty()).then(|| sum / row.len() as f32)
            })
            .collect();
        let (top, bottom) = (greens[0], greens[greens.len() - 1]);
        assert!(top > bottom, "top {top}, bottom {bottom}");
    }
}
