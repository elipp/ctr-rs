//! Text in the game's own fonts (`render::font`), for the menus: a [`CtrText`] is a line of
//! glyphs, each an image cut from a picture of its font in the text's gradient. A texel is drawn
//! as a whole number of the window's pixels ([`TexelSize`]), so the glyphs stay sharp, and taller
//! than wide, as retail's are.
//!
//! The fonts have capitals, digits and a little punctuation only: a name or a chat line, which
//! may have anything in it, is for Bevy's `Text`, in Inconsolata (the default font, see
//! `main.rs`), or in its bold cut with [`bold`].
//!
//! Both have a bold style: [`CtrText::bold`] for the game's fonts, which have no bold cut and are
//! drawn twice a texel apart, and [`bold`] for Bevy's text.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::camera::visibility::VisibilitySystems;
use bevy::ui::UiSystems;
use bevy::ui::widget::NodeImageMode;
use bevy::window::PrimaryWindow;
use render::font::{Font, FontAtlas, Gradient};

use crate::pad_glyph::{self, PadGlyph};

/// Inconsolata Bold, which [`register_bold`] puts in.
pub const BOLD: Handle<bevy::text::Font> = bevy::asset::uuid_handle!("3b6f1c52-8e0d-4a8f-9a4e-6c1d2f7b9e10");

/// Puts [`BOLD`] in the font assets.
pub fn register_bold(fonts: &mut Assets<bevy::text::Font>) {
    fonts
        .insert(
            BOLD.id(),
            bevy::text::Font::from_bytes(include_bytes!("../fonts/Inconsolata-Bold.ttf").to_vec()),
        )
        .expect("the bold font's handle is free");
}

/// The text's font in bold: Inconsolata Bold, at the same size and the rest as they were.
pub fn bold(font: TextFont) -> TextFont {
    TextFont {
        font: BOLD.into(),
        ..font
    }
}

/// The lines of the PS1's picture: the 2D art is drawn as tall against the window as it was
/// against them.
const SCREEN_LINES: u32 = 240;

/// How big a texel of the game's 2D art is drawn, in logical pixels, across and down. Down it is
/// as many whole physical pixels as the window's height fits the PS1's 240 lines into, and across
/// as many as keep it the shape of the HUD's pixels (`hud::PIXEL`); at least one each way.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TexelSize(pub Vec2);

impl TexelSize {
    /// `texels` of the 2D art across, as a UI length.
    pub fn x(&self, texels: usize) -> Val {
        Val::Px(texels as f32 * self.0.x)
    }

    /// `texels` of the 2D art down, as a UI length.
    pub fn y(&self, texels: usize) -> Val {
        Val::Px(texels as f32 * self.0.y)
    }
}

/// Both fonts, and a picture of each in the gradients asked for so far.
#[derive(Resource)]
pub struct Fonts {
    large: FontAtlas,
    middle: FontAtlas,
    images: HashMap<(Font, Gradient), Handle<Image>>,
}

impl Fonts {
    pub fn new(large: FontAtlas, middle: FontAtlas) -> Fonts {
        Fonts {
            large,
            middle,
            images: HashMap::new(),
        }
    }

    pub(crate) fn atlas(&self, font: Font) -> &FontAtlas {
        match font {
            Font::Large => &self.large,
            Font::Middle => &self.middle,
        }
    }

    /// The picture of `font` in `gradient`, made the first time it is asked for.
    fn image(
        &mut self,
        font: Font,
        gradient: Gradient,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let Fonts {
            large,
            middle,
            images: made,
        } = self;
        let atlas = match font {
            Font::Large => large,
            Font::Middle => middle,
        };
        made.entry((font, gradient))
            .or_insert_with(|| crate::hud::atlas_image(atlas.tinted(gradient), images))
            .clone()
    }
}

/// A line of text in one of the game's fonts. Its node is sized to the line, and its children are
/// the glyphs: it should have no others.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(Node)]
pub struct CtrText {
    pub text: String,
    pub font: Font,
    pub gradient: Gradient,
    /// The size of a texel in logical pixels, across and down, for a text that has to keep to
    /// fixed sizes (the HUD's). None follows the window ([`TexelSize`]).
    pub texel_size: Option<Vec2>,
    /// Each glyph drawn twice, the second a texel to the right, which the line is a texel per
    /// glyph wider for.
    pub bold: bool,
}

impl CtrText {
    pub fn new(text: impl Into<String>, font: Font, gradient: Gradient) -> CtrText {
        CtrText {
            text: text.into(),
            font,
            gradient,
            texel_size: None,
            bold: false,
        }
    }

    /// The text in bold.
    pub fn bold(mut self) -> CtrText {
        self.bold = true;
        self
    }

    /// The text with texels of `pixels` (across and down), whatever the window's size.
    pub fn fixed(mut self, pixels: Vec2) -> CtrText {
        self.texel_size = Some(pixels);
        self
    }
}

pub struct FontPlugin;

impl Plugin for FontPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TexelSize(Vec2::ONE)).add_systems(
            PostUpdate,
            (texel_size, lay_out)
                .chain()
                .before(UiSystems::Prepare)
                // Glyphs are respawned on every text change. Spawned after propagation, they'd
                // keep `InheritedVisibility(false)` for a frame: text blinks, a respawned menu
                // flashes.
                .before(VisibilitySystems::VisibilityPropagate)
                // Bevy leaves `Stack` unordered, and ours ran first: glyphs missing from the
                // stack draw at index 0, under their own panel, for a frame.
                .before(UiSystems::Stack),
        );
    }
}

/// Follows the window's height.
fn texel_size(windows: Query<&Window, With<PrimaryWindow>>, mut size: ResMut<TexelSize>) {
    let Ok(window) = windows.single() else {
        return;
    };
    let tall = (window.physical_height() / SCREEN_LINES).max(1);
    let aspect = crate::hud::PIXEL.y / crate::hud::PIXEL.x;
    let wide = (tall as f32 / aspect).round().max(1.0);
    size.set_if_neq(TexelSize(
        Vec2::new(wide, tall as f32) / window.scale_factor(),
    ));
}

/// A pad glyph's (`{cross}`) height, in its text's letters' heights. A keycap's (`{key:H}`) too.
const PAD_GLYPH_SCALE: f32 = 1.3;

/// A keycap's letters' size (Inconsolata Bold's em), in its heights.
const KEY_LETTER_SCALE: f32 = 0.55;
/// Inconsolata is monospaced, every character half an em wide: a key's name is measured without
/// laying it out.
const INCONSOLATA_ADVANCE: f32 = 0.5;
/// How wide a name fits on an unstretched keycap, in its heights; a wider one stretches it.
const KEYCAP_ROOM: f32 = 0.5;

/// What a `{name}` in a text is.
enum Inline<'a> {
    Pad(PadGlyph),
    /// A keycap with this on it.
    Key(&'a str),
}

impl Inline<'_> {
    fn parse(name: &str) -> Option<Inline<'_>> {
        match name.split_at_checked(4) {
            Some((key, label)) if key.eq_ignore_ascii_case("key:") && !label.is_empty() => {
                Some(Inline::Key(label))
            }
            _ => PadGlyph::parse(name).map(Inline::Pad),
        }
    }
}

/// Puts the glyphs of a text up again when it changed, or the texel size it follows did.
fn lay_out(
    mut commands: Commands,
    window: Res<TexelSize>,
    mut fonts: ResMut<Fonts>,
    mut images: ResMut<Assets<Image>>,
    mut texts: Query<(Entity, Ref<CtrText>, &mut Node)>,
) {
    for (entity, text, mut node) in &mut texts {
        if !text.is_changed() && !(window.is_changed() && text.texel_size.is_none()) {
            continue;
        }
        let size = TexelSize(text.texel_size.unwrap_or(window.0));
        let atlas = fonts.atlas(text.font);
        // A pad glyph is square in pixels, whatever shape the texels are.
        let (letter_top, letter_bottom) = atlas.letter_rows();
        let glyph_px = (letter_bottom - letter_top) as f32 * PAD_GLYPH_SCALE * size.0.y;
        let glyph_middle = (letter_top + letter_bottom) as f32 / 2.0 * size.0.y;
        let keycap_px = |label: &str| {
            let name = label.chars().count() as f32 * INCONSOLATA_ADVANCE * KEY_LETTER_SCALE;
            glyph_px * (1.0 + (name - KEYCAP_ROOM).max(0.0))
        };
        let mut inlines = Vec::new();
        let line = atlas.layout_with(&text.text, |name| {
            let inline = Inline::parse(name)?;
            let px = match inline {
                Inline::Pad(_) => glyph_px,
                Inline::Key(label) => keycap_px(label),
            };
            inlines.push(inline);
            Some((inlines.len() - 1, (px / size.0.x).ceil() as usize))
        });
        let image = fonts.image(text.font, text.gradient, &mut images);
        // Bold: a texel more for each glyph, its copy to the right.
        let thick = usize::from(text.bold);
        node.width = size.x(line.width + thick * line.glyphs.len());
        node.height = size.y(line.height);
        commands.entity(entity).despawn_related::<Children>();
        for (i, placed) in line.glyphs.iter().enumerate() {
            let glyph = placed.glyph;
            let left = (glyph.x + glyph.ink.0) as f32;
            // The copy first, so that the glyph's own left edge is over it.
            for shift in (0..=thick).rev() {
                commands.spawn((
                    ChildOf(entity),
                    ImageNode {
                        rect: Some(Rect::new(
                            left,
                            0.0,
                            left + glyph.ink_width() as f32,
                            glyph.height as f32,
                        )),
                        // Stretch: `Auto` would keep the glyph's own shape, with square texels, and
                        // fit it into the node (see `hud.rs`).
                        image_mode: NodeImageMode::Stretch,
                        ..ImageNode::new(image.clone())
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: size.x(placed.x + thick * i + shift),
                        top: Val::ZERO,
                        width: size.x(glyph.ink_width()),
                        height: size.y(glyph.height),
                        ..default()
                    },
                ));
            }
        }
        for placed in &line.boxes {
            let x = (placed.x + thick * placed.after_glyphs) as f32 * size.0.x;
            let inline = &inlines[placed.id];
            let (image, width) = match inline {
                Inline::Pad(glyph) => (
                    ImageNode {
                        rect: Some(glyph.rect()),
                        ..ImageNode::new(pad_glyph::ATLAS)
                    },
                    glyph_px,
                ),
                Inline::Key(label) => (
                    ImageNode {
                        rect: Some(pad_glyph::keycap_rect()),
                        image_mode: NodeImageMode::Sliced(pad_glyph::keycap_slicer()),
                        ..ImageNode::new(pad_glyph::ATLAS)
                    },
                    keycap_px(label),
                ),
            };
            let mut boxed = commands.spawn((
                ChildOf(entity),
                image,
                // Centred on the letters, sticking out of the line a little.
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(x + (placed.width as f32 * size.0.x - width) / 2.0),
                    top: Val::Px(glyph_middle - glyph_px / 2.0),
                    width: Val::Px(width),
                    height: Val::Px(glyph_px),
                    ..default()
                },
            ));
            if let Inline::Key(label) = inline {
                let face = pad_glyph::KEYCAP_FACE;
                boxed.with_child((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::ZERO,
                        right: Val::ZERO,
                        top: Val::Px(face.min.y * glyph_px),
                        height: Val::Px(face.height() * glyph_px),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    children![(
                        Text::new(*label),
                        bold(TextFont::from_font_size(KEY_LETTER_SCALE * glyph_px)),
                        TextColor(Color::BLACK),
                        TextLayout::no_wrap(),
                    )],
                ));
            }
        }
    }
}
