//! Reusable icon grid: Mask mode (cup wizard's weapons, later the bots' character pool) and Pick
//! mode (meant for replacing the text list in `drive/chooser.rs`).
//!
//! Stateless by design: the owner keeps the real state (e.g. `Config::masked_weapons`) and the
//! cursor, rebuilds [`Icons`] from it every frame, and the menu respawns the grid when `Icons`
//! compares unequal. Pointer input goes back through [`HasIconGrid`]; keys go through [`steer`],
//! called by the owner. So only the cursor frame and caption are patched in place ([`update`]).

use std::collections::HashMap;
use std::marker::PhantomData;

use bevy::ecs::component::Mutable;
use bevy::picking::events::{PointerClick, PointerMove, PointerRelease};
use bevy::picking::pointer::{PointerButton, PointerId};
use bevy::prelude::*;
use render::font::{Font, Gradient};
use render::mesh::Atlas;

use crate::font::{CtrText, TexelSize};
use crate::hud::{atlas_image, pack_icons};

const CURSOR: Color = Color::WHITE;
const PICKED: Color = Color::srgb(1.0, 0.8, 0.2);
const MASKED_ALPHA: f32 = 0.6;
const GREY_SHADE: f32 = 0.7;

/// A vertical strip of `cell`-sized icons (the layout `render`'s HUD atlases already use), plus
/// a CPU-made grey copy: `ImageNode::color` can only multiply, not desaturate.
pub struct Sheet {
    pub lit: Handle<Image>,
    pub grey: Handle<Image>,
    pub cell: UVec2,
}

impl Sheet {
    pub fn rect(&self, cell: usize) -> Rect {
        let top = (cell as u32 * self.cell.y) as f32;
        Rect::new(0.0, top, self.cell.x as f32, top + self.cell.y as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SheetId {
    /// Cells follow `render::weapons::ICON_IDS`; map items with `cell(icon_id(..))`.
    Weapons,
    /// Cells are character numbers (`render::portraits::ICON_IDS` order). Builds a second copy
    /// of the portraits, separate from `portrait_row::Portraits`; share it if memory matters.
    #[allow(dead_code, reason = "for the racer picker and the bots' pool, which are to come")]
    Portraits,
}

impl SheetId {
    /// Parses the racer pack each call: only from `Sheets::get`, which caches.
    fn atlas(self, vram: &loader::vram::Vram) -> (Atlas, UVec2) {
        let pack = pack_icons();
        match self {
            SheetId::Weapons => {
                use render::weapons::{HEIGHT, WIDTH, weapon_atlas};
                (weapon_atlas(&pack, vram), UVec2::new(WIDTH as u32, HEIGHT as u32))
            }
            SheetId::Portraits => {
                use render::portraits::{HEIGHT, WIDTH, portrait_atlas};
                (portrait_atlas(&pack, vram), UVec2::new(WIDTH as u32, HEIGHT as u32))
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct Sheets(HashMap<SheetId, Sheet>);

impl Sheets {
    pub fn get(
        &mut self,
        id: SheetId,
        vram: &loader::vram::Vram,
        images: &mut Assets<Image>,
    ) -> &Sheet {
        self.0.entry(id).or_insert_with(|| {
            let (atlas, cell) = id.atlas(vram);
            let grey = greyed(&atlas);
            Sheet {
                lit: atlas_image(atlas, images),
                grey: atlas_image(grey, images),
                cell,
            }
        })
    }
}

fn greyed(atlas: &Atlas) -> Atlas {
    let mut rgba = atlas.rgba.clone();
    for texel in rgba.chunks_exact_mut(4) {
        let [r, g, b] = [texel[0], texel[1], texel[2]].map(f32::from);
        let grey = ((0.299 * r + 0.587 * g + 0.114 * b) * GREY_SHADE).round() as u8;
        texel[..3].fill(grey);
    }
    Atlas {
        width: atlas.width,
        height: atlas.height,
        rgba,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Indexed like `Icons::cells`; `true` = masked.
    Mask(Vec<bool>),
    Pick(Option<usize>),
}

/// Compared each frame against what was spawned, so keep it cheap and free of handles.
#[derive(Clone, Debug, PartialEq)]
pub struct Icons {
    pub sheet: SheetId,
    pub cells: Vec<usize>,
    /// Same length as `cells`; shown as the caption at the cursor.
    pub names: Vec<String>,
    pub columns: usize,
    /// In `TexelSize` texels, not sheet pixels: previews draw smaller than the sheet's cells.
    pub size: UVec2,
    pub mode: Mode,
}

impl Icons {
    fn masked(&self, i: usize) -> bool {
        matches!(&self.mode, Mode::Mask(masked) if masked.get(i).copied().unwrap_or(false))
    }

    fn picked(&self, i: usize) -> bool {
        self.mode == Mode::Pick(Some(i))
    }
}

/// For configs that store a mask as a list of masked ids.
pub fn mask_of<T: PartialEq>(ids: &[T], masked: &[T]) -> Vec<bool> {
    ids.iter().map(|id| masked.contains(id)).collect()
}

pub fn toggle<T: PartialEq>(masked: &mut Vec<T>, id: T) {
    match masked.iter().position(|m| *m == id) {
        Some(at) => {
            masked.remove(at);
        }
        None => masked.push(id),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steered {
    Moved,
    Stayed,
    /// -1 top, 1 bottom. The owner moves its own selection; the cursor is left unchanged.
    Off(i32),
}

/// `way`: +x right, +y down (screen order, unlike the stick's +y up); only signs count, and y
/// wins over x. Across doesn't wrap rows: Left/Right at a row's end is `Stayed`.
pub fn steer(cursor: &mut usize, len: usize, columns: usize, way: IVec2) -> Steered {
    let columns = columns.max(1);
    if len == 0 {
        return if way.y != 0 { Steered::Off(way.y.signum()) } else { Steered::Stayed };
    }
    let (row, column) = (*cursor / columns, *cursor % columns);
    let rows = len.div_ceil(columns);
    if way.y != 0 {
        let to = row as i32 + way.y.signum();
        if to < 0 || to >= rows as i32 {
            return Steered::Off(way.y.signum());
        }
        *cursor = (to as usize * columns + column).min(len - 1);
        return Steered::Moved;
    }
    let to = column as i32 + way.x.signum();
    let in_row = (len - row * columns).min(columns);
    if way.x == 0 || to < 0 || to >= in_row as i32 {
        return Steered::Stayed;
    }
    *cursor = row * columns + to as usize;
    Steered::Moved
}

/// `from`: the `way.y` the selection moved by to land on the grid.
pub fn entered(len: usize, columns: usize, from: i32) -> usize {
    if from > 0 || len == 0 {
        0
    } else {
        (len - 1) / columns.max(1) * columns.max(1)
    }
}

/// `grid` is whatever id the owner passed to [`spawn`] (the pause menu uses the entry index).
/// The `Component` bound matches `HasAccordion`'s; `ResMut<R>` in observers needs it.
pub trait HasIconGrid: Resource + Component<Mutability = Mutable> {
    fn point_icon(&mut self, grid: usize, icon: usize);
    fn take_icon(&mut self, grid: usize, icon: usize);
}

#[derive(Component)]
pub struct GridIcon<R> {
    grid: usize,
    icon: usize,
    picked: bool,
    owner: PhantomData<fn() -> R>,
}

#[derive(Component)]
pub struct GridCaption<R> {
    grid: usize,
    names: Vec<String>,
    owner: PhantomData<fn() -> R>,
}

/// A non-`interactive` grid (preview) has no observers, frames or caption, so [`update`] never
/// touches it.
pub fn spawn<R: HasIconGrid>(
    commands: &mut Commands,
    parent: Entity,
    grid: usize,
    icons: &Icons,
    sheet: &Sheet,
    size: &TexelSize,
    interactive: bool,
) -> Entity {
    let gap = if interactive { 2 } else { 1 };
    let holder = commands
        .spawn((
            ChildOf(parent),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    let cells = commands
        .spawn((
            ChildOf(holder),
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::auto(icons.columns.max(1) as u16),
                column_gap: size.x(gap),
                row_gap: size.y(gap),
                ..default()
            },
        ))
        .id();
    for (i, &cell) in icons.cells.iter().enumerate() {
        let masked = icons.masked(i);
        let image = if masked { &sheet.grey } else { &sheet.lit };
        let alpha = if masked { MASKED_ALPHA } else { 1.0 };
        let picture = (
            ImageNode::new(image.clone())
                .with_rect(sheet.rect(cell))
                .with_color(Color::WHITE.with_alpha(alpha))
                .with_mode(NodeImageMode::Stretch),
            Node {
                width: size.x(icons.size.x as usize),
                height: size.y(icons.size.y as usize),
                ..default()
            },
            Pickable::IGNORE,
        );
        if !interactive {
            commands.spawn((ChildOf(cells), picture));
            continue;
        }
        let picked = icons.picked(i);
        let frame = commands
            .spawn((
                ChildOf(cells),
                GridIcon::<R> {
                    grid,
                    icon: i,
                    picked,
                    owner: PhantomData,
                },
                Node {
                    border: UiRect::axes(size.x(1), size.y(1)),
                    ..default()
                },
                BorderColor::all(if picked { PICKED } else { Color::NONE }),
            ))
            // Not PointerOver: a respawn under a still mouse fires Over and would steal the
            // cursor.
            .observe(move |_: On<PointerMove>, mut owner: ResMut<R>| {
                owner.point_icon(grid, i);
            })
            // Stop propagation: the pause menu's row cell would treat the click as taking the
            // grid entry, toggling the icon a second time.
            .observe(move |mut click: On<PointerClick>, mut owner: ResMut<R>| {
                click.propagate(false);
                if click.button == PointerButton::Primary {
                    owner.take_icon(grid, i);
                }
            })
            // A touch's pointer has no press on what it starts over, and so no click.
            .observe(move |mut release: On<PointerRelease>, mut owner: ResMut<R>| {
                if matches!(release.pointer.id, PointerId::Touch(_)) {
                    release.propagate(false);
                    owner.take_icon(grid, i);
                }
            })
            .id();
        commands.spawn((ChildOf(frame), picture));
    }
    if interactive {
        commands.spawn((
            ChildOf(holder),
            GridCaption::<R> {
                grid,
                names: icons.names.clone(),
                owner: PhantomData,
            },
            CtrText::new("", Font::Middle, Gradient::WHITE),
            Node {
                margin: UiRect::top(size.y(3)),
                ..default()
            },
        ));
    }
    holder
}

pub type Frames<'w, 's, R> = Query<'w, 's, (&'static GridIcon<R>, &'static mut BorderColor)>;
pub type Captions<'w, 's, R> =
    Query<'w, 's, (&'static GridCaption<R>, &'static mut CtrText), Without<GridIcon<R>>>;

/// `cursor`: `None` when the owner's selection isn't on a grid.
pub fn update<R: HasIconGrid>(
    cursor: Option<(usize, usize)>,
    frames: &mut Frames<R>,
    captions: &mut Captions<R>,
) {
    for (icon, mut border) in frames {
        let colour = if cursor == Some((icon.grid, icon.icon)) {
            CURSOR
        } else if icon.picked {
            PICKED
        } else {
            Color::NONE
        };
        if border.top != colour {
            *border = BorderColor::all(colour);
        }
    }
    for (caption, mut text) in captions {
        let name = match cursor {
            Some((grid, icon)) if grid == caption.grid => {
                caption.names.get(icon).map_or("", String::as_str)
            }
            _ => "",
        };
        if text.text != name {
            text.text = name.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_moves_within_rows_and_off_the_ends() {
        // 11 icons in rows of 4: the last row has 3.
        let mut at = 0;
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(-1, 0)), Steered::Stayed);
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(1, 0)), Steered::Moved);
        assert_eq!(at, 1);
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(0, -1)), Steered::Off(-1));
        at = 7;
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(1, 0)), Steered::Stayed);
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(0, 1)), Steered::Moved);
        // Below the 8th is nothing: the last icon.
        assert_eq!(at, 10);
        assert_eq!(steer(&mut at, 11, 4, IVec2::new(0, 1)), Steered::Off(1));
        assert_eq!(entered(11, 4, -1), 8);
        assert_eq!(entered(11, 4, 1), 0);
    }

    #[test]
    fn a_mask_flips() {
        let mut masked = vec![3u8];
        toggle(&mut masked, 4);
        toggle(&mut masked, 3);
        assert_eq!(masked, vec![4]);
        assert_eq!(mask_of(&[3, 4, 5], &masked), vec![false, true, false]);
    }
}
