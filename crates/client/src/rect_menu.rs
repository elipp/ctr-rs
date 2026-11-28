//! Menus of rows in the pause menu's panel (`pause::spawn_panel`), with retail's rows: the menu's
//! font, orange, and a bar behind the row picked whose green comes and goes (`MenuHighlight`).
//! Sizes are pixels of the PS1's picture ([`TexelSize`]).
//!
//! DEVIATION: retail adds the bar to what is under it; this lays it over, which is darker on light
//! parts of the picture (as `flash.rs`).

use std::marker::PhantomData;

use bevy::ecs::component::Mutable;
use bevy::picking::events::{PointerClick, PointerOver, PointerRelease};
use bevy::picking::pointer::{PointerButton, PointerId};
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::{CtrText, TexelSize};

/// How much of the bar is laid over the box (retail adds all of it).
const BAR_ALPHA: f32 = 0.9;

/// The rows' text (`baseFlags` 0).
pub const TEXT: Gradient = Gradient::ORANGE;

/// The height of a row of the small font (`font_charPixHeight[FONT_SMALL]`).
pub const ROW: usize = 8;
/// Between a subtitle and the rows (not retail).
const SUBTITLE_GAP: usize = 12;

/// The bar behind the row picked, at `seconds` (`MenuHighlight`: red 0x80, and a green that
/// swings 0x40 either side of 0x40 with a sine of the frame counter, `<< 7` a frame at 30 Hz).
pub fn bar(seconds: f32) -> Color {
    let turns = seconds * 30.0 * 128.0 / 4096.0;
    let green = 0x40 as f32 + 0x40 as f32 * (turns * std::f32::consts::TAU).sin();
    Color::srgba(0x80 as f32 / 255.0, green / 255.0, 0.0, BAR_ALPHA)
}

/// A menu's box (the pause menu's panel, `pause::spawn_panel`) with `title`, and `subtitle` under
/// it, put where `place` says (an absolute node's insets, which `place` sets). Returns the box and
/// the node its rows go in, one under another.
pub fn spawn(
    commands: &mut Commands,
    size: &TexelSize,
    title: &str,
    subtitle: Option<CtrText>,
    place: impl FnOnce(&mut Node),
) -> (Entity, Entity) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    place(&mut node);
    let root = commands.spawn(node).id();
    let panel = crate::pause::spawn_panel(commands, root, size, title);
    if let Some(subtitle) = subtitle {
        commands.spawn((
            ChildOf(panel),
            subtitle,
            Node {
                margin: UiRect::bottom(size.y(SUBTITLE_GAP)),
                ..default()
            },
        ));
    }
    // (Stretched: the panel centres its children, and a row may want its right edge.)
    let rows = commands
        .spawn((
            ChildOf(panel),
            Node {
                flex_direction: FlexDirection::Column,
                align_self: AlignSelf::Stretch,
                ..default()
            },
        ))
        .id();
    (root, rows)
}

/// The owner of a menu's items ([`spawn_items`]), which the pointer goes back to. The
/// `Component` bound is `HasIconGrid`'s: `ResMut<R>` in observers needs it.
pub trait HasMenuItems: Resource + Component<Mutability = Mutable> {
    fn point_item(&mut self, item: usize);
    fn take_item(&mut self, item: usize);
}

/// An item's box, which has the bar when it is the one picked ([`update_items`]).
#[derive(Component)]
pub struct MenuItem<R> {
    item: usize,
    owner: PhantomData<fn() -> R>,
}

/// An item's text.
#[derive(Component)]
pub struct MenuItemText<R> {
    item: usize,
    owner: PhantomData<fn() -> R>,
}

/// The items `labels` in `parent` (`spawn`'s rows), one under another or, `across`, side by side.
pub fn spawn_items<R: HasMenuItems>(
    commands: &mut Commands,
    parent: Entity,
    size: &TexelSize,
    labels: impl IntoIterator<Item = String>,
    across: bool,
) {
    let holder = commands
        .spawn((
            ChildOf(parent),
            Node {
                flex_direction: if across { FlexDirection::Row } else { FlexDirection::Column },
                column_gap: size.x(8),
                // (Across, the items are in the middle of the menu, whatever is under them.)
                align_self: if across { AlignSelf::Center } else { AlignSelf::Stretch },
                ..default()
            },
        ))
        .id();
    for (item, label) in labels.into_iter().enumerate() {
        commands
            .spawn((
                ChildOf(holder),
                MenuItem::<R> { item, owner: PhantomData },
                Node {
                    height: size.y(ROW),
                    padding: if across {
                        UiRect::horizontal(size.x(4))
                    } else {
                        UiRect::left(size.x(1))
                    },
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_child((
                MenuItemText::<R> { item, owner: PhantomData },
                CtrText::new(label, Font::Middle, TEXT),
                Pickable::IGNORE,
            ))
            .observe(move |_: On<PointerOver>, mut owner: ResMut<R>| owner.point_item(item))
            .observe(move |click: On<PointerClick>, mut owner: ResMut<R>| {
                if click.button == PointerButton::Primary {
                    owner.take_item(item);
                }
            })
            // A touch's pointer has no press on what it starts over, and so no click.
            .observe(move |release: On<PointerRelease>, mut owner: ResMut<R>| {
                if matches!(release.pointer.id, PointerId::Touch(_)) {
                    owner.take_item(item);
                }
            });
    }
}

/// The bar on the item `picked`, and the items' texts `label` says (patched only where changed).
pub fn update_items<R: HasMenuItems>(
    picked: usize,
    seconds: f32,
    label: impl Fn(usize) -> String,
    items: &mut Query<(&MenuItem<R>, &mut BackgroundColor)>,
    texts: &mut Query<(&MenuItemText<R>, &mut CtrText)>,
) {
    let bar = bar(seconds);
    for (item, mut color) in items {
        color.0 = if item.item == picked { bar } else { Color::NONE };
    }
    for (text, mut shown) in texts {
        let wanted = label(text.item);
        if shown.text != wanted {
            shown.text = wanted;
        }
    }
}
