//! Text in the game's own fonts (`render::font`), for the menus: a [`CtrText`] is a line of
//! glyphs, each an image cut from a picture of its font in the text's gradient. A texel is drawn
//! as a whole number of the window's pixels ([`TexelSize`]), so the glyphs stay sharp, and taller
//! than wide, as retail's are.
//!
//! The fonts have capitals, digits and a little punctuation only: a name or a chat line, which
//! may have anything in it, is for Bevy's `Text`.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::ui::UiSystems;
use bevy::window::PrimaryWindow;
use render::font::{Font, FontAtlas, Gradient};

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

    fn atlas(&self, font: Font) -> &FontAtlas {
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
            .or_insert_with(|| crate::leaderboard::atlas_image(atlas.tinted(gradient), images))
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
}

impl CtrText {
    pub fn new(text: impl Into<String>, font: Font, gradient: Gradient) -> CtrText {
        CtrText {
            text: text.into(),
            font,
            gradient,
            texel_size: None,
        }
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
            (texel_size, lay_out).chain().before(UiSystems::Prepare),
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
        let line = fonts.atlas(text.font).layout(&text.text);
        let image = fonts.image(text.font, text.gradient, &mut images);
        node.width = size.x(line.width);
        node.height = size.y(line.height);
        commands.entity(entity).despawn_related::<Children>();
        for placed in &line.glyphs {
            let glyph = placed.glyph;
            let left = (glyph.x + glyph.ink.0) as f32;
            commands.spawn((
                ChildOf(entity),
                ImageNode {
                    rect: Some(Rect::new(
                        left,
                        0.0,
                        left + glyph.ink_width() as f32,
                        glyph.height as f32,
                    )),
                    ..ImageNode::new(image.clone())
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: size.x(placed.x),
                    top: Val::ZERO,
                    width: size.x(glyph.ink_width()),
                    height: size.y(glyph.height),
                    ..default()
                },
            ));
        }
    }
}
