//! What the race's leaderboard (`leaderboard`) and the lobby's list (`lobby_list`) both draw: a
//! column of portraits (`render::portraits`), each with the player's name under it, the ping on
//! its right (which [`pings`] keeps up), and the host's "H" on a corner. Our name is in
//! [`HIGHLIGHT`]: with every character open to everyone, the same face can be in the column more
//! than once (which retail never allows), and this tells which one is ours. (Not retail.)

use bevy::prelude::*;
use render::font::{Font, Gradient};
use render::portraits::{HEIGHT, ICON_IDS, WIDTH};

use crate::font::CtrText;
use crate::hud;
use crate::hud::{atlas_image, pack_icons};
use crate::leaderboard::ICON_X;
use crate::lobby::Lobby;

/// The portraits as a picture, loaded when the first list or chat line needs it.
#[derive(Resource)]
pub(crate) struct Portraits(pub(crate) Handle<Image>);

/// The size of the names.
const FONT_SIZE: f32 = 16.0;

/// The size of the pings, and of the lobby's lines of those who only watch.
pub(crate) const PING_FONT_SIZE: f32 = 12.0;

/// What the names and the pings are on: the toasts' colour, more opaque so that it reads over the
/// portrait.
pub(crate) const BACKGROUND: Color = Color::srgba(0.1, 0.1, 0.1, 0.9);

/// The colour of our name (or, in a race, that of the kart the view follows), and of the "R" of a
/// relayed ping.
pub(crate) const HIGHLIGHT: Color = Color::srgb(1.0, 0.9, 0.2);

/// The colour of the "D" of a direct way: the green of the toasts of who joined.
const DIRECT: Color = Color::srgb(0.4, 1.0, 0.4);

/// The host's badge: a friendly yellow, pale at the top and golden at the bottom.
const HOST_GRADIENT: Gradient = Gradient {
    top: [0xff, 0xf4, 0x90],
    bottom: [0xf0, 0xc0, 0x30],
};

/// The ping of the player with a short key.
#[derive(Component)]
struct PingLabel(String);

/// The way to the player after its ping, with the debug HUD only: a green "D" for direct, which
/// is generally the better, and a yellow ([`HIGHLIGHT`]) "R" for a relay.
#[derive(Component)]
struct RouteSpan(String);

pub struct PortraitRowPlugin;

impl Plugin for PortraitRowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, pings.run_if(crate::drive::driving));
    }
}

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

/// Puts `character`'s portrait in `column`, at [`ICON_X`] and `y` lines of retail's screen down.
pub(crate) fn spawn_portrait(
    commands: &mut Commands,
    column: Entity,
    image: &Handle<Image>,
    character: u8,
    y: f32,
) -> Entity {
    commands
        .spawn((
            // Stretch: `Auto` would keep the picture's square texels, not the HUD's tall ones.
            ImageNode::new(image.clone())
                .with_rect(portrait_rect(character as usize))
                .with_mode(NodeImageMode::Stretch),
            Node {
                position_type: PositionType::Absolute,
                left: hud::x(ICON_X),
                top: hud::y(y),
                width: hud::x(WIDTH as f32),
                height: hud::y(HEIGHT as f32),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(column),
        ))
        .id()
}

/// Puts the host's "H" on a corner of its portrait: the top right, or the top left where the
/// race's leaderboard has its place there.
pub(crate) fn spawn_host_badge(commands: &mut Commands, portrait: Entity, right: bool) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        top: Val::ZERO,
        padding: UiRect::all(Val::Px(2.0)),
        ..default()
    };
    if right {
        node.right = Val::ZERO;
    } else {
        node.left = Val::ZERO;
    }
    commands.spawn((
        CtrText::new("H", Font::Middle, HOST_GRADIENT).fixed(hud::PIXEL / 2.0),
        BackgroundColor(BACKGROUND),
        node,
        Pickable::IGNORE,
        ChildOf(portrait),
    ));
}

/// Puts a player's name on a portrait, at the bottom and centered, and the ping (which [`pings`]
/// fills in) just right of the portrait, centered on it from top to bottom. Gives the name's
/// entity back.
pub(crate) fn spawn_tags(
    commands: &mut Commands,
    portrait: Entity,
    name: &str,
    key: &str,
) -> Entity {
    // The name's centered under the portrait, whatever its length.
    let wrapper = commands
        .spawn((
            UiTransform::from_translation(Val2::percent(-50.0, 0.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                // A little below the portrait's edge.
                bottom: Val::Px(-6.0),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id();
    let tag = commands
        .spawn((
            Text::new(name),
            TextFont {
                font_size: FONT_SIZE.into(),
                ..default()
            },
            BackgroundColor(BACKGROUND),
            Node {
                padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(wrapper),
        ))
        .id();
    let ping = commands
        .spawn((
            PingLabel(key.to_string()),
            Text::new(""),
            TextFont {
                font_size: PING_FONT_SIZE.into(),
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.8, 0.8)),
            BackgroundColor(BACKGROUND),
            // Up by half its own height, for its middle to be the portrait's, and a little left,
            // over the portrait's edge.
            UiTransform::from_translation(Val2::new(hud::x(-2.0), Val::Percent(-50.0))),
            Node {
                position_type: PositionType::Absolute,
                // From the portrait's right edge, halfway down it.
                left: Val::Percent(100.0),
                top: Val::Percent(50.0),
                padding: UiRect::axes(Val::Px(3.0), Val::Px(1.0)),
                display: Display::None,
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(portrait),
        ))
        .id();
    commands.spawn((
        RouteSpan(key.to_string()),
        TextSpan::new(""),
        TextFont {
            font_size: PING_FONT_SIZE.into(),
            ..default()
        },
        TextColor(HIGHLIGHT),
        ChildOf(ping),
    ));
    tag
}

/// Writes each player's ping, and shows the label once there is one (none for us, nor before
/// the first pong). With the debug HUD on (`Config::debug_hud`), the way to the player too (see
/// [`RouteSpan`]).
fn pings(
    lobby: Res<Lobby>,
    config: Res<crate::config::Config>,
    mut labels: Query<(&PingLabel, &mut Text, &mut Node)>,
    mut routes: Query<(&RouteSpan, &mut TextSpan, &mut TextColor)>,
) {
    for (RouteSpan(key), mut span, mut color) in &mut routes {
        let (route, tint) = match lobby.relayed.get(key).filter(|_| config.debug_hud) {
            Some(true) => (" R", HIGHLIGHT),
            Some(false) => (" D", DIRECT),
            None => ("", HIGHLIGHT),
        };
        if span.0 != route {
            span.0 = route.to_string();
        }
        if color.0 != tint {
            color.0 = tint;
        }
    }
    for (PingLabel(key), mut text, mut node) in &mut labels {
        let ping = lobby.pings.get(key);
        let line = ping.map_or(String::new(), |ping| format!("{} ms", ping.as_millis()));
        if text.0 != line {
            text.0 = line;
        }
        node.display = if ping.is_some() {
            Display::DEFAULT
        } else {
            Display::None
        };
    }
}
