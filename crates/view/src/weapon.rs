//! The weapon the driver holds, at the top left of the screen (`UI_Weapon_DrawSelf`). While the
//! roulette runs it shuffles through the weapons, a new one every other tick, and then shows
//! the one the sim settled on, with the number of shots when there are several.
//!
//! A driver with 10 wumpa fruit is juiced up: a glow pulses behind the fruit counter and, when
//! there is a weapon, behind the weapon (`UI_Weapon_DrawBG`).

use bevy::prelude::*;
use render::weapons::{
    HEIGHT, ICON_IDS, SHINE_IMAGE_HEIGHT, SHINE_IMAGE_WIDTH, SHINE_SCALE, WIDTH, cell, icon_id,
    shine_image, shine_pulse, weapon_atlas,
};
use vehicle::item::held;

use crate::leaderboard::{atlas_image, pack_icons};

/// The pictures of the weapons and of the glow, loaded when the first race needs them.
#[derive(Resource)]
struct WeaponIcons {
    weapons: Handle<Image>,
    shine: Handle<Image>,
}

/// A glow, behind the weapon or behind the wumpa counter.
#[derive(Component)]
enum Shine {
    Weapon,
    Wumpa,
}

/// The icon, with the quantity as its child.
#[derive(Component)]
struct WeaponHud;

#[derive(Component)]
struct Quantity;

/// How much larger than the PlayStation's pixels the icon is drawn.
const SCALE: f32 = 1.5;

/// Where the weapon icon's top left is.
const WEAPON_LEFT: f32 = 40.0;
const WEAPON_TOP: f32 = 48.0;

/// The glow's size on screen: the mirrored picture, shrunk as the game does.
const SHINE_SIZE: Vec2 = Vec2::new(
    SHINE_IMAGE_WIDTH as f32 * SHINE_SCALE * SCALE,
    SHINE_IMAGE_HEIGHT as f32 * SHINE_SCALE * SCALE,
);

/// `wumpaShineTheta`'s step: retail adds it each time it draws a glow, so a frame with two
/// glows turns it twice as far. The game has 30 frames a second.
const THETA_STEP: f32 = 0x100 as f32;
const FRAMES_PER_SECOND: f32 = 30.0;

/// The middle of the wumpa counter, the fruit and the count, from the top right corner of the screen.
const WUMPA_CENTER: Vec2 = Vec2::new(79.0, 32.0);

/// `UI_WEAPON_ROULETTE_RACING_COUNT`: the roulette shuffles through the first twelve items.
const ROULETTE_ITEMS: u32 = 12;

/// How many times a second the roulette picture changes: every other tick of the game.
const SHUFFLES_PER_SECOND: f32 = 15.0;

/// `DRIVER_WUMPA_JUICED_COUNT`
const JUICED: i8 = 10;

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show, shine).chain());
    }
}

/// The item the roulette shows at `step`: a scramble of the step, so that it does not go round in
/// order. (Retail takes `rand()`; this is only for show.) The spring turns into a turbo.
fn shuffled(step: u32) -> u8 {
    let item = (step.wrapping_mul(2_654_435_761) >> 16) % ROULETTE_ITEMS;
    if item == held::SPRING as u32 {
        held::TURBO
    } else {
        item as u8
    }
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    textures: Res<crate::Textures>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    icons: Option<Res<WeaponIcons>>,
    mut hud: Query<(&mut ImageNode, &mut Visibility, &Children), With<WeaponHud>>,
    mut quantity: Query<&mut Text, With<Quantity>>,
) {
    let driver = sim.driver();
    let Ok((mut image, mut visibility, children)) = hud.single_mut() else {
        // The first frame of a race, or of a new level (which took the nodes away).
        let atlas = match icons {
            Some(icons) => icons.weapons.clone(),
            None => {
                let pack = pack_icons();
                let icons = WeaponIcons {
                    weapons: atlas_image(weapon_atlas(&pack, &textures.vram), &mut images),
                    shine: atlas_image(shine_image(&pack, &textures.vram), &mut images),
                };
                let weapons = icons.weapons.clone();
                commands.insert_resource(icons);
                weapons
            }
        };
        let root = commands
            .spawn((
                WeaponHud,
                ImageNode::new(atlas),
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(WEAPON_LEFT),
                    top: Val::Px(WEAPON_TOP),
                    width: Val::Px(WIDTH as f32 * SCALE),
                    height: Val::Px(HEIGHT as f32 * SCALE),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .id();
        commands.spawn((
            Quantity,
            Text::new(""),
            TextFont {
                font_size: 28.0.into(),
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(-18.0),
                bottom: Val::Px(0.0),
                ..default()
            },
            ChildOf(root),
        ));
        return;
    };
    let shown = match driver.held_item_id {
        held::NONE => None,
        held::ROULETTE => {
            let step = (time.elapsed_secs() * SHUFFLES_PER_SECOND) as u32;
            Some(icon_id(shuffled(step), false, driver.character))
        }
        item => Some(icon_id(item, driver.num_wumpas >= JUICED, driver.character)),
    };
    let cell = shown.and_then(cell);
    *visibility = if cell.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if let Some(n) = cell {
        let top = (n * HEIGHT) as f32;
        image.rect = Some(Rect::new(0.0, top, WIDTH as f32, top + HEIGHT as f32));
    }
    let count = if driver.held_item_id == held::ROULETTE || driver.num_held_items == 0 {
        String::new()
    } else {
        driver.num_held_items.to_string()
    };
    for &child in children {
        if let Ok(mut text) = quantity.get_mut(child)
            && text.0 != count
        {
            text.0 = count.clone();
        }
    }
}

/// The glows behind the weapon and the wumpa counter, while the driver is juiced up and has not
/// finished.
fn shine(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    time: Res<Time<Real>>,
    icons: Option<Res<WeaponIcons>>,
    mut glows: Query<(&Shine, &mut ImageNode, &mut Visibility)>,
) {
    // The weapon's own system makes the pictures, on its first frame.
    let Some(icons) = icons else {
        return;
    };
    if glows.is_empty() {
        let glow = |shine: Shine, node: Node| {
            (
                shine,
                ImageNode::new(icons.shine.clone()),
                Visibility::Hidden,
                node,
                // Behind the counter and the weapon.
                GlobalZIndex(-1),
                Pickable::IGNORE,
            )
        };
        let size = |node: Node| Node {
            position_type: PositionType::Absolute,
            width: Val::Px(SHINE_SIZE.x),
            height: Val::Px(SHINE_SIZE.y),
            ..node
        };
        commands.spawn(glow(
            Shine::Weapon,
            size(Node {
                left: Val::Px(WEAPON_LEFT + WIDTH as f32 * SCALE / 2.0 - SHINE_SIZE.x / 2.0),
                top: Val::Px(WEAPON_TOP + HEIGHT as f32 * SCALE / 2.0 - SHINE_SIZE.y / 2.0),
                ..default()
            }),
        ));
        commands.spawn(glow(
            Shine::Wumpa,
            size(Node {
                right: Val::Px(WUMPA_CENTER.x - SHINE_SIZE.x / 2.0),
                top: Val::Px(WUMPA_CENTER.y - SHINE_SIZE.y / 2.0),
                ..default()
            }),
        ));
        return;
    }
    let driver = sim.driver();
    let juiced = driver.num_wumpas >= JUICED
        && driver.actions_flag_set & vehicle::actions::RACE_FINISHED == 0;
    let armed = juiced && driver.held_item_id != held::NONE;
    // The glow of the weapon turns the pulse twice as fast when both are up.
    let theta =
        time.elapsed_secs() * FRAMES_PER_SECOND * THETA_STEP * if armed { 2.0 } else { 1.0 };
    // 1 to 2 over the colours at their dimmest, which the picture is made at.
    let pulse = shine_pulse(theta as u32) / 2.0;
    for (shine, mut image, mut visibility) in &mut glows {
        let up = match shine {
            Shine::Weapon => armed,
            Shine::Wumpa => juiced,
        };
        *visibility = if up {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        image.color = Color::srgba(1.0, 1.0, 1.0, pulse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shuffle_only_shows_items_that_have_pictures() {
        for step in 0..2000 {
            let item = shuffled(step);
            assert!(item < ROULETTE_ITEMS as u8 && item != held::SPRING);
            assert!(cell(icon_id(item, false, 0)).is_some());
        }
        assert!(ICON_IDS.len() > ROULETTE_ITEMS as usize);
    }
}
