//! The pause menu, in the game's fonts. Escape, P or a pad's Start opens and closes it. Up and
//! Down (W and S, the D-pad) pick an item, Return or Space (Cross) takes it, and Backspace
//! (Triangle) goes back to the race. The mouse picks an item by pointing at it, and a click takes
//! it.
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

/// Whether the menu is open, and what it was asked to do.
#[derive(Resource, Default)]
pub struct PauseMenu {
    pub open: bool,
    /// The item picked, of those the menu shows.
    selected: usize,
    /// The item that was clicked, to take in the next update.
    clicked: Option<usize>,
    /// RESTART was taken: the next tick puts the race back on the grid.
    pub restart: bool,
}

/// Something the menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Resume,
    Restart,
    LeaveLobby,
    Quit,
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Resume => "Resume",
            Item::Restart => "Restart",
            Item::LeaveLobby => "Leave lobby",
            Item::Quit => "Quit",
        }
    }
}

/// What the menu offers now. An online race cannot be restarted by one player.
fn items(lobby: &Lobby) -> Vec<Item> {
    let mut items = vec![Item::Resume];
    if lobby.online.is_none() {
        items.push(Item::Restart);
    }
    if lobby.is_open() {
        items.push(Item::LeaveLobby);
    }
    items.push(Item::Quit);
    items
}

/// The panel behind the items, the line around it, and its shadow.
const PANEL: Color = Color::srgba(0.04, 0.04, 0.06, 0.75);
const BORDER: Color = Color::srgb(0.5, 0.85, 0.85);
const SHADOW: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The menu on screen, and what it was built for.
#[derive(Component)]
struct Shown {
    items: Vec<Item>,
    title: &'static str,
}

/// An item's text, and its place in the menu.
#[derive(Component)]
struct Row(usize);

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .add_systems(
                PreUpdate,
                input.after(InputSystems).after(crate::command::capture),
            )
            .add_systems(Update, show);
    }
}

/// Opens and closes the menu, moves the pick and takes an item.
fn input(
    mut menu: ResMut<PauseMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    lobby: Res<Lobby>,
    mut actions: MessageWriter<Action>,
    mut exit: MessageWriter<AppExit>,
) {
    let was_open = menu.open;
    let key = |codes: &[KeyCode]| codes.iter().any(|&k| keys.just_pressed(k));
    let pad = |button: GamepadButton| pads.iter().any(|p| p.just_pressed(button));
    if key(&[KeyCode::Escape, KeyCode::KeyP]) || pad(GamepadButton::Start) {
        menu.open = !menu.open;
        menu.selected = 0;
        menu.clicked = None;
    } else if menu.open {
        let items = items(&lobby);
        let count = items.len();
        if key(&[KeyCode::ArrowUp, KeyCode::KeyW]) || pad(GamepadButton::DPadUp) {
            menu.selected = (menu.selected + count - 1) % count;
        }
        if key(&[KeyCode::ArrowDown, KeyCode::KeyS]) || pad(GamepadButton::DPadDown) {
            menu.selected = (menu.selected + 1) % count;
        }
        let taken = if key(&[KeyCode::Enter, KeyCode::NumpadEnter, KeyCode::Space])
            || pad(GamepadButton::South)
        {
            Some(menu.selected)
        } else {
            menu.clicked.take()
        };
        if key(&[KeyCode::Backspace]) || pad(GamepadButton::North) {
            menu.open = false;
        }
        match taken.and_then(|i| items.get(i)) {
            Some(Item::Resume) => menu.open = false,
            Some(Item::Restart) => {
                menu.open = false;
                menu.restart = true;
            }
            Some(Item::LeaveLobby) => {
                menu.open = false;
                actions.write(Action::CloseLobby);
            }
            Some(Item::Quit) => {
                exit.write(AppExit::Success);
            }
            None => {}
        }
    }
    // The keys that opened, worked or closed the menu are not for anything else.
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
    size: Res<TexelSize>,
    shown: Query<(Entity, &Shown)>,
    rows: Query<(&Row, &Children)>,
    mut texts: Query<&mut CtrText>,
) {
    let wanted = menu.open.then(|| Shown {
        items: items(&lobby),
        // Online the race goes on.
        title: if lobby.online.is_none() {
            "Paused"
        } else {
            "Menu"
        },
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
    let title = shown.title;
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
                item.label(),
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
