//! The look of retail's menus (`RECTMENU_DrawSelf`, `RECTMENU_DrawInnerRect`): a box that darkens
//! what is under it to half, in a frame 3 pixels wide at the sides and 2 at the top and bottom,
//! with a shadow 12 pixels to the right and 6 down. A title in the large font is over a line in
//! the frame's colour, and the rows are in the menu's font, orange. The row picked has a bar
//! behind it whose green comes and goes (`MenuHighlight`). Sizes are pixels of the PS1's picture
//! ([`TexelSize`]).
//!
//! DEVIATION: retail adds the frame and the bar to what is under them; these lay them over, which
//! is darker on light parts of the picture (as `flash.rs`).

use std::marker::PhantomData;

use bevy::ecs::component::Mutable;
use bevy::picking::events::{PointerClick, PointerOver, PointerRelease};
use bevy::picking::pointer::{PointerButton, PointerId};
use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::{CtrText, TexelSize};

/// `battleSetup_Color_UI_1`: the frame, and the line under a title.
const FRAME: Color = Color::srgb(0.0, 0x50 as f32 / 255.0, 0x40 as f32 / 255.0);
/// `DrawSolidBoxData[2]` and `[0]` drawn half over the picture: the inside and the shadow.
const INSIDE: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);
/// How much of the bar is laid over the box (retail adds all of it).
const BAR_ALPHA: f32 = 0.9;

/// The rows' text (`baseFlags` 0).
pub const TEXT: Gradient = Gradient::ORANGE;

/// The frame's width at the sides, and at the top and bottom (`RECTMENU_DrawOuterRect_HighLevel`).
const FRAME_X: usize = 3;
const FRAME_Y: usize = 2;
/// How far the box reaches past the text, across and down (`RECTMENU_DrawSelf`'s `borders`).
const MARGIN_X: usize = 6;
const MARGIN_Y: usize = 4;
/// How far the shadow is to the right and down.
const SHADOW_X: usize = 12;
const SHADOW_Y: usize = 6;
/// The height of a row of the small font (`font_charPixHeight[FONT_SMALL]`), of the large font's
/// letters (`[FONT_BIG]`), and the gap under a large title, before the line.
pub const ROW: usize = 8;
const TITLE: usize = 17;
const TITLE_GAP: usize = 6;
/// The line under a title, and the gap under it.
const LINE: usize = 2;
const LINE_GAP: usize = 1;

/// The bar behind the row picked, at `seconds` (`MenuHighlight`: red 0x80, and a green that
/// swings 0x40 either side of 0x40 with a sine of the frame counter, `<< 7` a frame at 30 Hz).
pub fn bar(seconds: f32) -> Color {
    let turns = seconds * 30.0 * 128.0 / 4096.0;
    let green = 0x40 as f32 + 0x40 as f32 * (turns * std::f32::consts::TAU).sin();
    Color::srgba(0x80 as f32 / 255.0, green / 255.0, 0.0, BAR_ALPHA)
}

/// Where a menu's title is over its line.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TitleAlign {
    Left,
    Center,
}

/// A menu's box, with a large `title` over its line, put where `place` says (an absolute node's
/// insets, which `place` sets). Returns the box and the node its rows go in, one under another.
pub fn spawn(
    commands: &mut Commands,
    size: &TexelSize,
    title: &str,
    align: TitleAlign,
    place: impl FnOnce(&mut Node),
) -> (Entity, Entity) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        ..default()
    };
    place(&mut node);
    // The box and its shadow, which is two pieces so as not to darken the box twice.
    let root = commands.spawn(node).id();
    commands.spawn((
        ChildOf(root),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(100.0),
            top: size.y(SHADOW_Y),
            width: size.x(SHADOW_X),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(INSIDE),
        Pickable::IGNORE,
    ));
    commands.spawn((
        ChildOf(root),
        Node {
            position_type: PositionType::Absolute,
            left: size.x(SHADOW_X),
            right: Val::ZERO,
            top: Val::Percent(100.0),
            height: size.y(SHADOW_Y),
            ..default()
        },
        BackgroundColor(INSIDE),
        Pickable::IGNORE,
    ));
    let inside = commands
        .spawn((
            ChildOf(root),
            Node {
                flex_direction: FlexDirection::Column,
                border: UiRect::axes(size.x(FRAME_X), size.y(FRAME_Y)),
                padding: UiRect::axes(size.x(MARGIN_X - FRAME_X), size.y(MARGIN_Y - FRAME_Y)),
                ..default()
            },
            BackgroundColor(INSIDE),
            BorderColor::all(FRAME),
        ))
        .id();
    commands.spawn((
        ChildOf(inside),
        CtrText::new(title, Font::Large, TEXT),
        Node {
            margin: UiRect::bottom(size.y(TITLE + TITLE_GAP - Font::Large.line_height())),
            align_self: match align {
                TitleAlign::Left => AlignSelf::FlexStart,
                TitleAlign::Center => AlignSelf::Center,
            },
            ..default()
        },
    ));
    // The line runs from frame to frame.
    commands.spawn((
        ChildOf(inside),
        Node {
            height: size.y(LINE),
            margin: UiRect {
                left: -size.x(FRAME_X),
                right: -size.x(FRAME_X),
                bottom: size.y(LINE_GAP),
                ..default()
            },
            ..default()
        },
        BackgroundColor(FRAME),
        Pickable::IGNORE,
    ));
    let rows = commands
        .spawn((
            ChildOf(inside),
            Node {
                flex_direction: FlexDirection::Column,
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
