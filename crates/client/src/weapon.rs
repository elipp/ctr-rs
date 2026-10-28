//! The weapon the driver holds, at the top middle of the screen (`UI_Weapon_DrawSelf`). While the
//! roulette runs it shuffles through the weapons, a new one every other tick, and then shows
//! the one the sim settled on, with the number of shots when there are several. The icons carry
//! their own box, which the HUD draws half-transparent as retail does
//! (`render::weapons::weapon_atlas`).
//!
//! A driver with 10 wumpa fruit is juiced up: a glow pulses behind the wumpa fruit (not the count) and, when
//! there is a weapon, behind the weapon (`UI_Weapon_DrawBG`).

use bevy::prelude::*;
use render::weapons::{
    HEIGHT, SHINE_IMAGE_HEIGHT, SHINE_IMAGE_WIDTH, SHINE_SCALE, WIDTH, cell, icon_id, shine_image,
    shine_pulse, weapon_atlas,
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

/// How much larger than the PlayStation's pixels the HUD is drawn.
const SCALE: f32 = 1.5;

/// How much larger than that the weapon (its icon and its glow) is drawn, and how far lower than
/// retail's slot it is, in pixels.
const WEAPON_ZOOM: f32 = 1.2;
const WEAPON_LOWER: f32 = 10.0;
const WEAPON_SCALE: f32 = SCALE * WEAPON_ZOOM;

/// How much taller than wide a retail pixel of the HUD is, which makes the icon (48 by 32 texels)
/// come out a little taller than wide. Measured, not derived: the bomb icon's frame is 133 by 139
/// pixels in a retail screenshot, and 132 by 93 here at the same width with square pixels, so
/// the height needs 1.48 times more. (A 512 by 256 picture shown at 4:3 would give 1.5, and the
/// 3D view's 216 lines `exhaust::SCREEN_PIXEL_ASPECT`'s 1.78: the HUD is not drawn in that.)
const HUD_PIXEL_ASPECT: f32 = 1.5;

/// Where the weapon icon's top is: retail's slot is at `y` 5 (and, like the size, stretched by the
/// pixel's shape). It is centered across the screen, as retail's is (the slot's `x` of 232 and the
/// icon's 48 texels make 256, the middle of 512).
const WEAPON_TOP: f32 = 5.0 * WEAPON_SCALE * HUD_PIXEL_ASPECT + WEAPON_LOWER;

/// The icon's size on screen.
const WEAPON_SIZE: Vec2 = Vec2::new(
    WIDTH as f32 * WEAPON_SCALE,
    HEIGHT as f32 * WEAPON_SCALE * HUD_PIXEL_ASPECT,
);

/// The glow's size on screen at a HUD scale: the mirrored picture, shrunk as the game does.
const fn shine_at(scale: f32) -> Vec2 {
    Vec2::new(
        SHINE_IMAGE_WIDTH as f32 * SHINE_SCALE * scale,
        SHINE_IMAGE_HEIGHT as f32 * SHINE_SCALE * scale * HUD_PIXEL_ASPECT,
    )
}

/// How much smaller the wumpa fruit's glow is than the HUD's glow at the plain scale.
const WUMPA_SHINE_SCALE: f32 = 0.5;

/// The glow's size on screen, for the weapon or the fruit.
fn shine_size(shine: &Shine) -> Vec2 {
    match shine {
        Shine::Weapon => shine_at(WEAPON_SCALE),
        Shine::Wumpa => shine_at(SCALE) * WUMPA_SHINE_SCALE,
    }
}

/// `wumpaShineTheta`'s step: retail adds it each time it draws a glow, so a frame with two
/// glows turns it twice as far. The game has 30 frames a second.
const THETA_STEP: f32 = 0x100 as f32;
const FRAMES_PER_SECOND: f32 = 30.0;

/// How many times a second the roulette picture changes: every other tick of the game.
const SHUFFLES_PER_SECOND: f32 = 15.0;

/// `DRIVER_WUMPA_JUICED_COUNT`
const JUICED: i8 = 10;

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show, shine).chain().run_if(crate::drive::driving));
    }
}

/// The item the roulette shows at `step`: a scramble of the step, so that it does not go round in
/// order. (Retail takes `rand()`; this is only for show.) Only the items it can land on.
fn shuffled(step: u32) -> u8 {
    let enabled = &vehicle::item::ENABLED;
    enabled[((step.wrapping_mul(2_654_435_761) >> 16) as usize) % enabled.len()]
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
                // Stretch: `Auto` would keep the picture's own proportions inside the node.
                ImageNode::new(atlas).with_mode(NodeImageMode::Stretch),
                Visibility::Hidden,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(50.0),
                    margin: UiRect::left(Val::Px(-WEAPON_SIZE.x / 2.0)),
                    top: Val::Px(WEAPON_TOP),
                    width: Val::Px(WEAPON_SIZE.x),
                    height: Val::Px(WEAPON_SIZE.y),
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
    // After a use the icon strobes, gone every other tick (and its count with it) until the
    // cooldown is over.
    let strobed = driver.held_item_id != held::ROULETTE
        && driver.no_item_timer != 0
        && sim.race.tick.is_multiple_of(2);
    *visibility = if cell.is_some() && !strobed {
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
    mut glows: Query<(
        Entity,
        &Shine,
        &mut ImageNode,
        &mut Visibility,
        Option<&UiTargetCamera>,
    )>,
    cameras: Query<(Entity, &Camera), With<Camera3d>>,
) {
    // The weapon's own system makes the pictures, on its first frame.
    let Some(icons) = icons else {
        return;
    };
    if glows.is_empty() {
        let glow = |shine: Shine, node: Node| {
            (
                shine,
                ImageNode::new(icons.shine.clone()).with_mode(NodeImageMode::Stretch),
                Visibility::Hidden,
                node,
                // Behind the counter and the weapon.
                GlobalZIndex(-1),
                Pickable::IGNORE,
            )
        };
        let size = |shine: &Shine, node: Node| Node {
            position_type: PositionType::Absolute,
            width: Val::Px(shine_size(shine).x),
            height: Val::Px(shine_size(shine).y),
            ..node
        };
        let weapon = shine_size(&Shine::Weapon);
        commands.spawn(glow(
            Shine::Weapon,
            size(
                &Shine::Weapon,
                Node {
                    left: Val::Percent(50.0),
                    margin: UiRect::left(Val::Px(-weapon.x / 2.0)),
                    top: Val::Px(WEAPON_TOP + WEAPON_SIZE.y / 2.0 - weapon.y / 2.0),
                    ..default()
                },
            ),
        ));
        let wumpa = shine_size(&Shine::Wumpa);
        commands.spawn(glow(
            Shine::Wumpa,
            size(
                &Shine::Wumpa,
                // Behind the fruit only, not the count.
                Node {
                    left: Val::Percent(50.0),
                    margin: UiRect::left(Val::Px(
                        crate::wumpa_icon::FRUIT_X * crate::hud::PIXEL.x - wumpa.x / 2.0,
                    )),
                    top: Val::Px(crate::wumpa_icon::MIDDLE_Y * crate::hud::PIXEL.y - wumpa.y / 2.0),
                    ..default()
                },
            ),
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
    // The UI is drawn by the camera of the highest order, which is the fruit's own: the glows
    // would be drawn over the fruit and blend with it. They are drawn by the main camera instead,
    // under the cameras that draw the fruit (and the kart).
    let main = cameras.iter().find(|(_, c)| c.order == 0).map(|(e, _)| e);
    for (entity, shine, mut image, mut visibility, target) in &mut glows {
        if let Some(main) = main
            && target.is_none_or(|t| t.0 != main)
        {
            commands.entity(entity).insert(UiTargetCamera(main));
        }
        let up = match shine {
            Shine::Weapon => armed,
            Shine::Wumpa => juiced,
        };
        *visibility = if up {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        // The pulse is the glow's brightness; its alpha is the picture's.
        image.color = Color::srgb(pulse, pulse, pulse);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shuffle_only_shows_items_that_have_pictures() {
        for step in 0..2000 {
            let item = shuffled(step);
            assert!(vehicle::item::ENABLED.contains(&item));
            assert!(cell(icon_id(item, false, 0)).is_some());
        }
    }
}
