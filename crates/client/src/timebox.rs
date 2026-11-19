//! A relic race's time crate counter (`UI_DrawNumTimebox`): the level's 1-second time crate
//! spinning on its own (`UI_ThTick_CountPickup`), a small orange "x" and "broken/all" in the big
//! orange digits, at the top of the screen left of the middle. Drawn by the HUD camera of the
//! wumpa counter's fruit.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use loader::model::Model;
use render::font::{Font, Gradient};
use render::mesh;

use crate::font::CtrText;
use crate::hud;
use crate::hud_view::{HudView, HudViews};
use crate::subtract::SubtractMaterial;
use crate::wumpa_icon::HudModel;
use crate::{Textures, Track};

/// `hud_1P_P1[UI_HUD_SLOT_TIMEBOX]`'s line, which the "x" and the count are placed from. Across,
/// the crate and the count are centred together on the screen: retail's x of 200 is that for a
/// count as wide as "01/65".
const SLOT_Y: f32 = 30.0;

/// The line of the middle of the crate as drawn: measured from a retail screenshot, where it is a
/// little above the middle of the "x".
const CRATE_MIDDLE_Y: f32 = 21.3;

/// How tall the crate's own sides are, in lines, so that tilted it is as tall as in a retail
/// screenshot (about 20.7 lines). Half as wide in pixels as tall in lines, as the HUD's pixels
/// are 2:3.
const CRATE_LINES: f32 = 17.0;
const CRATE_HALF_WIDTH: f32 = CRATE_LINES * hud::PIXEL.y / hud::PIXEL.x / 2.0;

/// How far from below the crate is seen. Retail's HUD camera has perspective, and the slot is
/// 78 lines above the middle of its screen (0x6c) at a depth of 256: `atan(78 / 256)`. (Its
/// `rotateToHud` turns the model by that too, which by the screenshots does not undo it.)
const TILT: f32 = 0.2961;

/// `UI_REWARD_PICKUP_ROT_SLOW`: the crate turns 0x40 of 0x1000 a frame, half as fast as the
/// wumpa display.
const SPIN_TICKS: f32 = 64.0;

/// `UI_DrawNumTimebox`: the "x" and the count, from the crate's place (its middle across).
const X_AT: Vec2 = Vec2::new(0x14 as f32, -10.0);
const COUNT_AT: Vec2 = Vec2::new(0x21 as f32, -0xe as f32);

/// How far in front of the HUD camera it hangs, in pixels: anything between its near and far plane.
const DEPTH: f32 = 100.0;

/// The turning crate; its child is the model, its middle on the pivot.
#[derive(Component)]
struct TimeboxIcon {
    /// The model's height in world units, to size it for the screen.
    height: f32,
}

/// The "x" and the count, of a view, placed from the middle of the crate.
#[derive(Component)]
struct TimeboxCounter;

/// Where the middle of the crate is across, in pixels of retail's screen from its middle, so that
/// the crate and a count `count_width` pixels wide are centred together.
fn crate_x(count_width: f32) -> f32 {
    let width = CRATE_HALF_WIDTH + COUNT_AT.x + count_width;
    CRATE_HALF_WIDTH - width / 2.0
}

/// `%2.02d/%ld`: the crates broken, of the level's.
#[derive(Component)]
struct TimeboxCount;

pub struct TimeboxPlugin;

impl Plugin for TimeboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (build, show).chain().distributive_run_if(crate::drive::driving),
        );
    }
}

/// Puts the crate and the counter up on the first frame of a level, which takes the old ones away
/// with it. The crate is the level's (`gGT->modelPtr[STATIC_TIME_CRATE_01]`): a level without one
/// has the counter alone, as retail does.
#[allow(clippy::too_many_arguments)]
fn build(
    mut commands: Commands,
    track: Res<Track>,
    textures: Res<Textures>,
    hud: HudViews,
    icons: Query<&HudView, With<TimeboxIcon>>,
    counters: Query<&HudView, With<TimeboxCounter>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut none: Local<bool>,
) {
    for view in hud.missing(counters.iter().copied()) {
        let at = |offset: Vec2| Node {
            position_type: PositionType::Absolute,
            left: hud::x(offset.x),
            top: hud::y(SLOT_Y + offset.y),
            ..default()
        };
        commands.spawn((
            TimeboxCounter,
            HudView(view),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            children![
                (
                    CtrText::new("x", Font::Middle, Gradient::ORANGE).fixed(hud::PIXEL),
                    at(X_AT),
                ),
                (
                    TimeboxCount,
                    HudView(view),
                    CtrText::new("", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
                    at(COUNT_AT),
                ),
            ],
        ));
    }

    if track.is_changed() {
        *none = false;
    }
    let missing = hud.missing(icons.iter().copied());
    if missing.is_empty() || *none {
        return;
    }
    let found = track
        .0
        .instances
        .iter()
        .find(|i| i.model_id == instance::lev_init::TIME_CRATE_01)
        .and_then(|inst| {
            let header = Model::parse_at(&textures.dram, inst.model)
                .ok()?
                .headers
                .into_iter()
                .next()?;
            let frame = crate::props::rest_pose(&header)?.clone();
            Some((header, frame))
        });
    let Some((header, frame)) = found else {
        *none = true;
        return;
    };
    // (Retail's scale is the HUD slot's alone, not the level's for its crates.)
    let model = HudModel::new(
        mesh::model_mesh(&header, &frame, &textures.vram, [0x1000; 3]),
        &mut images,
    );
    let middle = (model.low + model.high) / 2.0;
    for view in missing {
        let pivot = commands
            .spawn((
                TimeboxIcon {
                    height: model.high.y - model.low.y,
                },
                HudView(view),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        model.spawn(
            Transform::from_translation(-middle),
            pivot,
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut subtract_materials,
        );
    }
}

/// Shows the crate and the counter in a relic race, counts the crates, centres both on the view
/// and turns the crate.
#[allow(clippy::too_many_arguments)]
fn show(
    sim: Res<crate::props::Sim>,
    race_on: Res<crate::race_on::RaceOn>,
    hud: HudViews,
    fonts: Res<crate::font::Fonts>,
    between: crate::between::Between,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut icons: Query<
        (&TimeboxIcon, &HudView, &mut Transform, &mut Visibility),
        Without<TimeboxCounter>,
    >,
    mut counters: Query<(&HudView, &mut Node, &mut Visibility), With<TimeboxCounter>>,
    mut counts: Query<(&HudView, &mut CtrText), With<TimeboxCount>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let race = &*sim.race;
    let on = race_on.relic();
    let visibility = if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (.., mut v) in &mut counters {
        v.set_if_neq(visibility);
    }
    for (.., mut v) in &mut icons {
        v.set_if_neq(visibility);
    }
    if !on {
        return;
    }
    let all = race.time_crates();
    let angle =
        (race.tick as f32 + between.fraction()) / SPIN_TICKS * std::f32::consts::TAU;
    for kart in hud.iter() {
        let count = format!("{:02}/{all}", kart.driver.num_time_crates);
        let width = fonts.atlas(Font::Large).layout(&count).width as f32;
        let x = crate_x(width);
        for (_, mut text) in counts.iter_mut().filter(|(v, _)| v.0 == kart.view) {
            if text.text != count {
                text.text = count.clone();
            }
        }
        for (_, mut node, _) in counters.iter_mut().filter(|(v, ..)| v.0 == kart.view) {
            let margin = UiRect::left(hud::x(x));
            if node.margin != margin {
                node.margin = margin;
            }
        }
        // The HUD camera looks at the middle of the window, with y up.
        let rect = kart.rect(window);
        for (icon, _, mut transform, _) in icons.iter_mut().filter(|(_, v, ..)| v.0 == kart.view) {
            transform.translation = Vec3::new(
                rect.center().x - window.width() / 2.0 + x * hud::PIXEL.x,
                window.height() / 2.0 - rect.min.y - CRATE_MIDDLE_Y * hud::PIXEL.y,
                -DEPTH,
            );
            transform.scale = Vec3::splat(CRATE_LINES * hud::PIXEL.y / icon.height);
            // Spun on its own axis, then tipped towards us at the bottom.
            transform.rotation = Quat::from_rotation_x(-TILT) * Quat::from_rotation_y(angle);
        }
    }
}
