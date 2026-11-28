//! What the race's leaderboard (`leaderboard`) and the lobby's list (`lobby_list`) both draw: a
//! column of portraits (`render::portraits`), each with the player's name under it and the
//! host's crown on a corner. (The pings are the scoreboard's, `scoreboard.rs`.) Our name is in
//! [`HIGHLIGHT`]: with every character open to everyone, the same face can be in the column more
//! than once (which retail never allows), and this tells which one is ours. (Not retail.)

use bevy::prelude::*;
use render::portraits::{HEIGHT, ICON_IDS, WIDTH};

use crate::hud;
use crate::hud::{atlas_image, pack_icons};

/// The portraits as a picture, loaded when the first list or chat line needs it.
#[derive(Resource)]
pub(crate) struct Portraits(pub(crate) Handle<Image>);

/// The size of the names.
const FONT_SIZE: f32 = 16.0;

/// The size of the lobby's lines of those who only watch.
pub(crate) const PING_FONT_SIZE: f32 = 12.0;

/// What the names are on: the toasts' colour, more opaque so that it reads over the
/// portrait.
pub(crate) const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.9);

/// The colour of our name (or, in a race, that of the kart the view follows), and of the
/// scoreboard's "R" of a relayed ping.
pub(crate) const HIGHLIGHT: Color = Color::srgb(1.0, 0.9, 0.2);

/// The colour of the scoreboard's "D" of a direct way: the green of the toasts of who joined.
pub(crate) const DIRECT: Color = crate::toast::SUCCESS_COLOR;

/// The lobby's watchers' and free places' grey.
pub(crate) const DIM: Color = Color::srgb(0.8, 0.8, 0.8);

/// Around a name's text, in its [`tag`].
pub(crate) const TAG_PADDING: UiRect = UiRect::axes(Val::Px(4.0), Val::Px(1.0));

/// A name's text on [`BACKGROUND`]; its node (with [`TAG_PADDING`] for a name) is the caller's.
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

/// The host's crown, golden, and its size in pixels. Outlined as the names over the karts are,
/// which reads over any portrait without a box.
const HOST_COLOR: Color = Color::srgb_u8(0xf8, 0xd0, 0x40);
const HOST_SIZE: f32 = 18.0;

/// The tick on the portrait of a quick play player who is ready for the next race: the green
/// of [`DIRECT`], the crown's size.
const READY_COLOR: Color = DIRECT;
const READY_SIZE: f32 = HOST_SIZE;

pub(crate) fn load_portraits(
    vram: &loader::vram::Vram,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    atlas_image(render::portraits::portrait_atlas(&pack_icons(), vram), images)
}

/// The portraits' picture: the one loaded already, or a new one, which is kept as [`Portraits`].
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

/// Where `character`'s portrait is in [`Portraits`] (an unknown one gets the last).
pub(crate) fn portrait_rect(character: usize) -> Rect {
    let character = character.min(ICON_IDS.len() - 1);
    Rect::new(
        0.0,
        (character * HEIGHT) as f32,
        WIDTH as f32,
        ((character + 1) * HEIGHT) as f32,
    )
}

/// Where a portrait goes: its top left corner `at`, in pixels and lines of retail's screen, and
/// `scale` times its size (the race's leaderboard: 1, as retail).
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

/// Puts `character`'s portrait in `column` (see [`place`]).
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

/// Puts the host's crown on a corner of its portrait: the top right, or the top left where the
/// race's leaderboard has its place there.
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

/// Puts the ready tick on the top left corner of a portrait, clear of the host's crown (which
/// a quick play lobby doesn't show anyway).
pub(crate) fn spawn_ready_badge(commands: &mut Commands, portrait: Entity) {
    let node = Node {
        position_type: PositionType::Absolute,
        top: Val::ZERO,
        left: Val::ZERO,
        margin: UiRect::all(Val::Px(2.0)),
        ..default()
    };
    commands.spawn((
        crate::icons::outlined(crate::icons::CHECK_FAT, READY_SIZE, READY_COLOR, node),
        Pickable::IGNORE,
        ChildOf(portrait),
    ));
}

/// How wide [`spawn_empty_portrait`]'s frame is, of the portrait's place: the faces' art is
/// narrower than its picture, which a frame the picture's width shows up.
const EMPTY_WIDTH: f32 = 0.88;

/// Puts a place that no player has taken yet in `column`, where [`spawn_portrait`] would: a
/// frame ([`EMPTY_WIDTH`], centered) with a bold question mark in it. Gives back the place, the
/// portrait's size, for its name to go on as a portrait's does.
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

/// The size of [`spawn_empty_portrait`]'s question mark.
const EMPTY_MARK_SIZE: f32 = 28.0;

/// Puts a name on a portrait (or an empty place), and gives its entity back.
pub(crate) fn spawn_name(commands: &mut Commands, portrait: Entity, name: &str) -> Entity {
    commands
        .spawn((
            tag(name, FONT_SIZE, Color::WHITE),
            TextLayout::linebreak(LineBreak::NoWrap),
            // Inside the cell: below it, the tag would cover the next row's top and make the
            // rows run together. On the left edge: a name is usually wider than its portrait,
            // and centered it would stick out of the column (the lobby's title is on that edge).
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                bottom: Val::ZERO,
                padding: TAG_PADDING,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id()
}
