//! Phosphor's filled icons (MIT): an icon is a `Text` of its code point in [`FONT`]. The code
//! points are Phosphor's own, in its private use area.

use bevy::prelude::*;

/// Phosphor, cut down to the icons below (`just icons`, which a new one has to be added to), which
/// [`register`] puts in.
pub const FONT: Handle<Font> = bevy::asset::uuid_handle!("1aaf8033-bdaa-4c68-8850-e66a1ba7af3f");

pub const VIDEO_CAMERA: char = '\u{E4DA}';
pub const CROWN_SIMPLE: char = '\u{E616}';
pub const CHECK_CIRCLE: char = '\u{E184}';

/// Puts [`FONT`] in the font assets.
pub fn register(fonts: &mut Assets<Font>) {
    fonts
        .insert(
            FONT.id(),
            Font::from_bytes(include_bytes!("../fonts/Phosphor-Fill-subset.ttf").to_vec()),
        )
        .expect("the icon font's handle is free");
}

/// An [`outlined`] icon's outline, in pixels: thicker than the nameplates' (`names.rs`), which
/// is thin on an icon's solid shape.
const OUTLINE_WIDTH: f32 = 3.0;

/// The icon of an [`outlined`] one, over its outline.
#[derive(Component)]
pub struct Face;

/// `icon` in `color` with the nameplates' outline (`names.rs`), only thicker: copies of it in the outline's
/// colour under it, each moved the outline's width one way. Only the node itself is picked, not
/// the copies, so that the pointer goes over and out of it once; [`Face`] is the icon to recolour.
/// `node` is the outlined icon's own (its margins, say). It must have no padding: the copies are
/// placed from the padding's outer edge, the icon inside it.
pub fn outlined(icon: char, size: f32, color: Color, node: Node) -> impl Bundle {
    use crate::names::{OUTLINE_COLOR, OUTLINE_DIRECTIONS};
    let outline = OUTLINE_DIRECTIONS.map(move |direction| {
        (
            self::icon(icon, size, OUTLINE_COLOR),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(direction.x * OUTLINE_WIDTH),
                top: Val::Px(direction.y * OUTLINE_WIDTH),
                ..default()
            },
            Pickable::IGNORE,
        )
    });
    (
        node,
        Children::spawn((
            SpawnIter(outline.into_iter()),
            Spawn((Face, self::icon(icon, size, color), Pickable::IGNORE)),
        )),
    )
}

/// `icon`, `size` pixels tall, in `color`.
pub fn icon(icon: char, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(icon),
        TextFont {
            font: FONT.into(),
            ..TextFont::from_font_size(size)
        },
        TextColor(color),
    )
}
