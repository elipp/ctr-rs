//! The pause menu, in the game's fonts. Escape, P or a pad's Start opens and closes it. Up and
//! Down (W and S, the D-pad) pick an item, Return or Space (Cross) takes it, and Backspace
//! (Triangle) goes back to the race. The mouse picks an item by pointing at it, and a click takes
//! it.
//!
//! Outside a lobby it also offers "Public lobbies": the realm's list (`public_lobbies.rs`), where
//! taking a lobby joins it. Backspace goes back from there to the menu, not to the race.
//!
//! Offline the race stops while the menu is open. Online it goes on, as the others are still
//! racing: the menu only takes our controls, and our kart gets no input until it closes.
//!
//! While it is open it takes the keyboard, as the command line does: its keys do not drive the
//! kart, open the command line or switch the level.

use bevy::input::InputSystems;
use bevy::picking::events::{PointerClick, PointerOver};
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::command::Action;
use crate::font::{CtrText, TexelSize};
use crate::lobby::Lobby;
use crate::public_lobbies::{self, PublicLobbies, Status};
use crate::sound::{MENU_BACK_SFX, MENU_MOVE_SFX, MENU_TAKE_SFX, Sounds};

/// Whether the menu is open, and what it was asked to do.
#[derive(Resource, Default)]
pub struct PauseMenu {
    pub open: bool,
    /// What the menu shows: its own items, or the realm's lobbies.
    view: View,
    /// The item picked, of those the menu shows.
    selected: usize,
    /// The item that was clicked, to take in the next update.
    clicked: Option<usize>,
    /// RESTART was taken: the next tick puts the race back on the grid.
    pub restart: bool,
}

/// What the menu is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    Main,
    /// The public lobbies.
    Browser,
}

/// Something the menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Resume,
    Restart,
    LeaveLobby,
    /// Shows the realm's list.
    PublicLobbies,
    Quit,
    /// The public lobby at that place of [`PublicLobbies::shown`].
    Join(usize),
    Refresh,
    /// Back to the main view.
    Back,
    /// A line that says something and does nothing.
    Info,
}

/// An item and the text it has.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    item: Item,
    label: String,
}

fn entry(item: Item, label: impl Into<String>) -> Entry {
    Entry {
        item,
        label: label.into(),
    }
}

/// The title of the menu and what it offers now. An online race cannot be restarted by one
/// player.
fn items(lobby: &Lobby, view: View, public: &PublicLobbies) -> (String, Vec<Entry>) {
    if view == View::Browser {
        return ("Public lobbies".into(), browser_items(public));
    }
    let mut items = vec![entry(Item::Resume, "Resume")];
    if lobby.online.is_none() {
        items.push(entry(Item::Restart, "Restart"));
    }
    if lobby.is_open() {
        items.push(entry(Item::LeaveLobby, "Leave lobby"));
    } else {
        items.push(entry(Item::PublicLobbies, "Public lobbies"));
    }
    // A page cannot close its tab: in a browser quitting would only stop the game, on its last
    // frame.
    if cfg!(not(target_arch = "wasm32")) {
        items.push(entry(Item::Quit, "Quit"));
    }
    // Online the race goes on.
    let title = if lobby.online.is_none() { "Paused" } else { "Menu" };
    (title.into(), items)
}

/// The realm's lobbies, one to a row: the name, the players, and if it is not simply a lobby we
/// can join, why not.
fn browser_items(public: &PublicLobbies) -> Vec<Entry> {
    let mut items = Vec::new();
    match &public.status {
        Status::Idle | Status::Loading => items.push(entry(Item::Info, "Looking...")),
        Status::Failed => {
            items.push(entry(Item::Info, "Could not reach the realm"));
            items.push(entry(Item::Refresh, "Try again"));
        }
        Status::Loaded(lobbies) => {
            let (shown, more) = public.shown();
            if lobbies.is_empty() {
                items.push(entry(Item::Info, "No public lobbies"));
            }
            for (i, lobby) in shown.iter().enumerate() {
                let name: String = lobby.name.chars().take(NAME_SHOWN).collect();
                let note = if !lobby.compatible {
                    "  other version"
                } else if lobby.racing {
                    "  racing"
                } else {
                    ""
                };
                let label = format!("{name}  {}/{}{note}", lobby.players, lobby.max_players);
                items.push(entry(Item::Join(i), label));
            }
            if more > 0 {
                items.push(entry(Item::Info, format!("and {more} more")));
            }
            items.push(entry(Item::Refresh, "Refresh"));
        }
    }
    items.push(entry(Item::Back, "Back"));
    items
}

/// The most chars of a lobby's name a row shows: the font is wide, and the screen is not.
const NAME_SHOWN: usize = 20;

/// The panel behind the items, the line around it, and its shadow.
const PANEL: Color = Color::srgba(0.04, 0.04, 0.06, 0.75);
const BORDER: Color = Color::srgb(0.5, 0.85, 0.85);
const SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The menu on screen, and what it was built for.
#[derive(Component)]
struct Shown {
    items: Vec<Entry>,
    title: String,
}

/// An item's text, and its place in the menu.
#[derive(Component)]
struct Row(usize);

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .init_resource::<PublicLobbies>()
            .add_systems(Update, public_lobbies::poll.run_if(crate::drive::driving))
            .add_systems(
                PreUpdate,
                input
                    .after(InputSystems)
                    .after(crate::command::capture)
                    .run_if(crate::drive::driving),
            )
            .add_systems(Update, show.run_if(crate::drive::driving));
    }
}

/// Opens and closes the menu, moves the pick and takes an item, with the menu's sounds. Offline
/// it pauses the race's sounds too (`MainFreeze_IfPressStart`, `MainFrame_TogglePauseAudio`).
#[allow(clippy::too_many_arguments)]
fn input(
    mut menu: ResMut<PauseMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    lobby: Res<Lobby>,
    mut public: ResMut<PublicLobbies>,
    sounds: Option<Res<Sounds>>,
    mut actions: MessageWriter<Action>,
    mut exit: MessageWriter<AppExit>,
) {
    let was_open = menu.open;
    let offline = lobby.online.is_none();
    let sound = |id| {
        if let Some(sounds) = &sounds {
            sounds.menu(id);
        }
    };
    let key = |codes: &[KeyCode]| codes.iter().any(|&k| keys.just_pressed(k));
    let pad = |button: GamepadButton| pads.iter().any(|p| p.just_pressed(button));
    if key(&[KeyCode::Escape, KeyCode::KeyP]) || pad(GamepadButton::Start) {
        menu.open = !menu.open;
        menu.view = View::Main;
        menu.selected = 0;
        menu.clicked = None;
        if let Some(sounds) = sounds.as_ref().filter(|_| offline) {
            if menu.open {
                sounds.pause();
            } else {
                sounds.unpause();
            }
        }
        sound(MENU_TAKE_SFX);
        return finish(was_open, &menu, &mut keys);
    } else if menu.open {
        let (_, items) = items(&lobby, menu.view, &public);
        let count = items.len();
        if key(&[KeyCode::ArrowUp, KeyCode::KeyW]) || pad(GamepadButton::DPadUp) {
            menu.selected = (menu.selected + count - 1) % count;
            sound(MENU_MOVE_SFX);
        }
        if key(&[KeyCode::ArrowDown, KeyCode::KeyS]) || pad(GamepadButton::DPadDown) {
            menu.selected = (menu.selected + 1) % count;
            sound(MENU_MOVE_SFX);
        }
        let taken = if key(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space])
            || pad(GamepadButton::South)
        {
            Some(menu.selected)
        } else {
            menu.clicked.take()
        };
        if taken.is_some() {
            sound(MENU_TAKE_SFX);
        }
        if key(&[KeyCode::Backspace]) || pad(GamepadButton::North) {
            sound(MENU_BACK_SFX);
            if menu.view == View::Browser {
                menu.view = View::Main;
                menu.selected = 0;
            } else {
                menu.open = false;
            }
        }
        match taken.and_then(|i| items.get(i)).map(|e| e.item) {
            Some(Item::Resume) => menu.open = false,
            Some(Item::Restart) => {
                menu.open = false;
                menu.restart = true;
            }
            Some(Item::LeaveLobby) => {
                menu.open = false;
                actions.write(Action::CloseLobby);
            }
            Some(Item::PublicLobbies) => {
                menu.view = View::Browser;
                menu.selected = 0;
                public.refresh();
            }
            Some(Item::Refresh) => public.refresh(),
            Some(Item::Back) => {
                menu.view = View::Main;
                menu.selected = 0;
            }
            Some(Item::Join(i)) => {
                if let Some(found) = public.shown().0.get(i) {
                    actions.write(Action::JoinLobby(net::Ticket {
                        id: found.id,
                        relay: None,
                    }));
                    menu.open = false;
                }
            }
            Some(Item::Info) | None => {}
            Some(Item::Quit) => {
                exit.write(AppExit::Success);
            }
        }
        // Leaving the pause cuts the sound of what left it (`MainFrame_TogglePauseAudio`).
        if !menu.open
            && let Some(sounds) = sounds.as_ref().filter(|_| offline)
        {
            sounds.unpause();
        }
    }
    finish(was_open, &menu, &mut keys);
}

/// The keys that opened, worked or closed the menu are not for anything else.
fn finish(was_open: bool, menu: &PauseMenu, keys: &mut ButtonInput<KeyCode>) {
    if was_open || menu.open {
        keys.reset_all();
    }
}

/// Builds the menu when it opens (and again when what it offers or the texel size changes), takes
/// it down when it closes, and shows the pick in white.
fn show(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    lobby: Res<Lobby>,
    public: Res<PublicLobbies>,
    size: Res<TexelSize>,
    shown: Query<(Entity, &Shown)>,
    rows: Query<(&Row, &Children)>,
    mut texts: Query<&mut CtrText>,
) {
    let wanted = menu.open.then(|| {
        let (title, items) = items(&lobby, menu.view, &public);
        Shown { items, title }
    });
    let current = shown.single().ok();
    let stale = match (&wanted, current) {
        (Some(wanted), Some((_, current))) => {
            size.is_changed() || wanted.items != current.items || wanted.title != current.title
        }
        (None, None) => false,
        _ => true,
    };
    if stale {
        if let Some((entity, _)) = current {
            commands.entity(entity).despawn();
        }
        if let Some(wanted) = wanted {
            menu.selected = menu.selected.min(wanted.items.len() - 1);
            spawn(&mut commands, wanted, menu.selected, &size);
        }
        return;
    }
    for (row, children) in &rows {
        let gradient = gradient(row.0 == menu.selected);
        for &child in children {
            if let Ok(mut text) = texts.get_mut(child)
                && text.gradient != gradient
            {
                text.gradient = gradient;
            }
        }
    }
}

/// The picked item is white, the others orange.
fn gradient(picked: bool) -> Gradient {
    if picked {
        Gradient::WHITE
    } else {
        Gradient::ORANGE
    }
}

fn spawn(commands: &mut Commands, shown: Shown, selected: usize, size: &TexelSize) {
    let items = shown.items.clone();
    let title = shown.title.clone();
    // The whole window, to put the panel in its middle. Under the command line.
    let root = commands
        .spawn((
            shown,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(i32::MAX - 1),
        ))
        .id();
    let panel = commands
        .spawn((
            ChildOf(root),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                padding: UiRect::axes(size.x(12), size.y(6)),
                border: UiRect::axes(size.x(1), size.y(1)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(BORDER),
            BoxShadow::new(SHADOW, size.x(4), size.y(4), Val::ZERO, Val::ZERO),
        ))
        .id();
    commands.spawn((
        ChildOf(panel),
        CtrText::new(title, Font::Large, Gradient::ORANGE),
        Node {
            margin: UiRect::bottom(size.y(4)),
            ..default()
        },
    ));
    for (i, item) in items.into_iter().enumerate() {
        commands
            .spawn((
                ChildOf(panel),
                Row(i),
                Node {
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    padding: UiRect::vertical(size.y(2)),
                    ..default()
                },
            ))
            .with_child(CtrText::new(
                item.label,
                Font::Middle,
                gradient(i == selected),
            ))
            .observe(move |_: On<PointerOver>, mut menu: ResMut<PauseMenu>| {
                menu.selected = i;
            })
            .observe(
                move |click: On<PointerClick>, mut menu: ResMut<PauseMenu>| {
                    if click.button == PointerButton::Primary {
                        menu.clicked = Some(i);
                    }
                },
            );
    }
}
