//! Portrait rows shared by the race's leaderboard and the lobby's list. Our name is in
//! [`HIGHLIGHT`]: every character is open to everyone, so the same face can appear more than once
//! (retail never allows that).
use bevy::prelude::*;
use render::portraits::{HEIGHT, ICON_IDS, WIDTH};

use crate::hud;
use crate::hud::{atlas_image, pack_icons};

/// Loaded lazily by whichever list or chat line needs it first.
#[derive(Resource)]
pub(crate) struct Portraits(pub(crate) Handle<Image>);

const FONT_SIZE: f32 = 16.0;

pub(crate) const PING_FONT_SIZE: f32 = 12.0;

/// The toasts' colour, more opaque to read over a portrait.
pub(crate) const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.9);

pub(crate) const HIGHLIGHT: Color = Color::srgb(1.0, 0.9, 0.2);

pub(crate) const DIRECT: Color = crate::toast::SUCCESS_COLOR;

pub(crate) const DIM: Color = Color::srgb(0.8, 0.8, 0.8);

pub(crate) const TAG_PADDING: UiRect = UiRect::axes(Val::Px(4.0), Val::Px(1.0));

/// The node is the caller's.
pub(crate) fn tag(
    text: impl Into<String>,
    size: f32,
    color: Color,
) -> (Text, TextFont, TextColor, BackgroundColor) {
    (
        Text::new(text),
        TextFont::from_font_size(size),
        TextColor(color),
        BackgroundColor(BACKGROUND),
    )
}

/// Outlined like the names over the karts: reads over any portrait without a box.
pub(crate) const HOST_COLOR: Color = Color::srgb_u8(0xf8, 0xd0, 0x40);
const HOST_SIZE: f32 = 18.0;

pub(crate) const READY_COLOR: Color = DIRECT;

pub(crate) fn load_portraits(
    vram: &loader::vram::Vram,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    atlas_image(render::portraits::portrait_atlas(&pack_icons(), vram), images)
}

pub(crate) fn portraits(
    commands: &mut Commands,
    loaded: Option<&Portraits>,
    vram: &loader::vram::Vram,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    match loaded {
        Some(portraits) => portraits.0.clone(),
        None => {
            let handle = load_portraits(vram, images);
            commands.insert_resource(Portraits(handle.clone()));
            handle
        }
    }
}

/// An unknown character gets the last portrait.
pub(crate) fn portrait_rect(character: usize) -> Rect {
    let character = character.min(ICON_IDS.len() - 1);
    Rect::new(
        0.0,
        (character * HEIGHT) as f32,
        WIDTH as f32,
        ((character + 1) * HEIGHT) as f32,
    )
}

/// `at` is in retail's pixels and lines.
fn place(at: Vec2, scale: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: hud::x(at.x),
        top: hud::y(at.y),
        width: hud::x(WIDTH as f32 * scale),
        height: hud::y(HEIGHT as f32 * scale),
        ..default()
    }
}

pub(crate) fn spawn_portrait(
    commands: &mut Commands,
    column: Entity,
    image: &Handle<Image>,
    character: u8,
    at: Vec2,
    scale: f32,
) -> Entity {
    commands
        .spawn((
            // Stretch: `Auto` would keep the picture's square texels, not the HUD's tall ones.
            ImageNode::new(image.clone())
                .with_rect(portrait_rect(character as usize))
                .with_mode(NodeImageMode::Stretch),
            place(at, scale),
            Pickable::IGNORE,
            ChildOf(column),
        ))
        .id()
}

/// `right: false` for the race's leaderboard, whose place number is on the top left.
pub(crate) fn spawn_host_badge(commands: &mut Commands, portrait: Entity, right: bool) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        top: Val::ZERO,
        margin: UiRect::all(Val::Px(2.0)),
        ..default()
    };
    if right {
        node.right = Val::ZERO;
    } else {
        node.left = Val::ZERO;
    }
    commands.spawn((
        crate::icons::outlined(crate::icons::CROWN_SIMPLE, HOST_SIZE, HOST_COLOR, node),
        Pickable::IGNORE,
        ChildOf(portrait),
    ));
}

/// The faces' art is narrower than the portrait picture; a full-width frame looks too wide.
const EMPTY_WIDTH: f32 = 0.88;

/// Returns the portrait-sized place, so callers can hang things on it as on a portrait.
pub(crate) fn spawn_empty_portrait(commands: &mut Commands, column: Entity, at: Vec2, scale: f32) -> Entity {
    let place = commands
        .spawn((place(at, scale), Pickable::IGNORE, ChildOf(column)))
        .id();
    let frame = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent((1.0 - EMPTY_WIDTH) * 50.0),
                width: Val::Percent(EMPTY_WIDTH * 100.0),
                height: Val::Percent(100.0),
                border: UiRect::all(Val::Px(2.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(DIM),
            BackgroundColor(BACKGROUND),
            Pickable::IGNORE,
            ChildOf(place),
        ))
        .id();
    commands.spawn((
        Text::new("?"),
        crate::font::bold(TextFont::from_font_size(EMPTY_MARK_SIZE)),
        TextColor(DIM),
        Pickable::IGNORE,
        ChildOf(frame),
    ));
    place
}

const EMPTY_MARK_SIZE: f32 = 20.0;

pub(crate) fn spawn_name(commands: &mut Commands, portrait: Entity, name: &str) -> Entity {
    commands
        .spawn((
            tag(name, FONT_SIZE, Color::WHITE),
            TextLayout::linebreak(LineBreak::NoWrap),
            // Inside the cell: below it, the tag would cover the next row's top. Centered, a
            // name over ~170px wide runs off the screen's left past ICON_X.
            UiTransform::from_translation(Val2::percent(-50.0, 0.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                bottom: Val::ZERO,
                padding: TAG_PADDING,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id()
}
