//! The end of a solo run: a mockup of retail's relic ceremony (`RR_EndEvent_DrawMenu`, overlay
//! 223) with the same news but none of its layout or timing, shown for a time trial too (less the
//! relic's lines). The time, the perfect bonus, the relic the run earned (the pack's spinning
//! relic in the tier's colour) and the personal best (new or kept), centred on the screen in place
//! of the TIME block, from the settled finish (`CurrentRace::solo_end`) until the race ends
//! [`TICKS`] after it.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use loader::model::Model;
use race::relic::Tier;
use render::font::{Font, Gradient};
use render::mesh;

use crate::font::CtrText;
use crate::hud;
use crate::hud_view::{HudView, HudViews};
use crate::race_on::{RaceOn, SoloEnd, Submit};
use crate::subtract::SubtractMaterial;
use crate::wumpa_icon::HudModel;
use crate::Textures;

/// `STATIC_RELIC`
pub const RELIC_MODEL: i16 = 0x61;

/// How long the race goes on after its finish for the ceremony.
pub const TICKS: u32 = 10 * crate::GAME_TICK_HZ as u32;

/// The relic of the racers' pack.
#[derive(Resource)]
pub struct RelicModel(pub Model);

/// Where the panel's first line is, and the relic's middle, in lines of retail's screen.
const TOP: f32 = 40.0;
const RELIC_Y: f32 = 170.0;

/// How tall the relic is grown, in lines, and the ticks it grows for.
const RELIC_LINES: f32 = 50.0;
const GROW_TICKS: f32 = 24.0;

/// `UI_REWARD_PICKUP_ROT_SLOW`: a turn in 64 ticks (`UI_ThTick_Reward`).
pub(crate) const SPIN_TICKS: f32 = 64.0;

/// `OtherFX` of the relic unlocked.
const RELIC_SFX: u16 = 0x67;

/// How far in front of the HUD camera the relic hangs, in pixels.
const DEPTH: f32 = 100.0;

/// The panel of a view.
#[derive(Component)]
struct Panel;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Line {
    Time,
    Perfect,
    Relic,
    Best,
    OldBest,
    Submit,
}

/// The relic's pivot; its child is the model, its middle on the pivot.
#[derive(Component)]
struct Relic {
    tier: Tier,
    /// The model's height in world units.
    height: f32,
}

pub struct SoloEndPlugin;

impl Plugin for SoloEndPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (panel, relic).distributive_run_if(crate::drive::driving),
        );
    }
}

/// The solo run's finish, once settled.
pub fn shown(race_on: &RaceOn) -> Option<SoloEnd> {
    race_on.get().and_then(|race| race.solo_end)
}

/// `UI_INSTANCE_BirthWithThread`'s colours of the relic (`colorRGBA`), and `RR_EndEvent_DrawMenu`'s
/// gold and platinum.
fn tint(tier: Tier) -> [u8; 3] {
    match tier {
        Tier::Sapphire => [0x60, 0xa5, 0xff],
        Tier::Gold => [0xd8, 0xd2, 0x09],
        Tier::Platinum => [0xff, 0xed, 0xe9],
    }
}

#[allow(clippy::too_many_arguments)]
fn panel(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    race_on: Res<RaceOn>,
    config: Res<crate::config::Config>,
    pads: crate::drive::Pads,
    hud: HudViews,
    panels: Query<(&HudView, &mut Visibility), With<Panel>>,
    mut lines: Query<(&Line, &mut CtrText, &mut Node)>,
) {
    let mut panels = panels;
    for view in hud.missing(panels.iter().map(|(v, _)| *v)) {
        let line = |line: Line| {
            (
                line,
                CtrText::new("", Font::Large, Gradient::ORANGE).fixed(hud::PIXEL),
                Node::default(),
            )
        };
        commands.spawn((
            Panel,
            HudView(view),
            Node {
                position_type: PositionType::Absolute,
                left: Val::ZERO,
                right: Val::ZERO,
                top: hud::y(TOP),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: hud::y(6.0),
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            children![
                line(Line::Time),
                line(Line::Perfect),
                line(Line::Relic),
                line(Line::Best),
                line(Line::OldBest),
                line(Line::Submit)
            ],
        ));
    }
    let end = shown(&race_on);
    let visibility = if end.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (_, mut v) in &mut panels {
        v.set_if_neq(visibility);
    }
    let Some(end) = end else {
        return;
    };
    let slot = race_on.get().map_or(0, |race| race.viewed());
    let total = sim.race.final_ms(slot).unwrap_or(0);
    // Retail's text flashes orange and white, a frame each.
    let flash = if sim.race.tick % 2 == 0 {
        Gradient::ORANGE
    } else {
        Gradient::WHITE
    };
    for (line, mut text, mut node) in &mut lines {
        let (wanted, gradient) = match line {
            Line::Time => (
                Some(format!("YOUR TIME {}", crate::laps::clock(total))),
                Gradient::ORANGE,
            ),
            Line::Perfect => (end.perfect.then(|| "PERFECT -10".to_string()), flash),
            Line::Relic => (
                end.relic.map(|tier| format!("{} RELIC AWARDED!", crate::laps::tier_name(tier).to_uppercase())),
                end.relic.map_or(flash, crate::laps::tier_gradient),
            ),
            Line::Best => (end.new_best.then(|| "NEW HIGH SCORE!".to_string()), flash),
            Line::OldBest => (
                end.best.map(|ms| format!("BEST {}", crate::laps::clock(ms))),
                Gradient::WHITE,
            ),
            Line::Submit => (
                submit_line(end.submit, &pads, &config),
                match end.submit {
                    Some(Submit::Offered) => Gradient::YELLOW,
                    Some(Submit::Refused) => Gradient::RED,
                    _ => Gradient::WHITE,
                },
            ),
        };
        let display = if wanted.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let wanted = wanted.unwrap_or_default();
        if text.text != wanted {
            text.text = wanted;
        }
        if text.gradient != gradient {
            text.gradient = gradient;
        }
    }
}

/// (The font has no '#'.)
fn submit_line(
    submit: Option<Submit>,
    pads: &crate::drive::Pads,
    config: &crate::config::Config,
) -> Option<String> {
    Some(match submit? {
        Submit::Offered => {
            format!("{}: SUBMIT", crate::ghost::select_label(pads, config)?.to_uppercase())
        }
        Submit::Sending => "SUBMITTING...".to_string(),
        Submit::Accepted { rank, top_ms } => crate::scores::placed(rank, top_ms).to_uppercase(),
        Submit::Refused => "NOT SUBMITTED".to_string(),
    })
}

/// Puts the relic up when the run earned one (in its tier's colour: the model's own colours are
/// its normals, for retail's lighting, which this leaves out), grows it and turns it.
#[allow(clippy::too_many_arguments)]
fn relic(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    race_on: Res<RaceOn>,
    model: Res<RelicModel>,
    textures: Res<Textures>,
    sounds: Res<crate::sound::Sounds>,
    fixed: Res<Time<Fixed>>,
    hud: HudViews,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut relics: Query<(Entity, &Relic, &HudView, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    let earned = shown(&race_on).and_then(|end| Some((end.at, end.relic?)));
    for (entity, relic, ..) in &relics {
        if earned.is_none_or(|(_, tier)| tier != relic.tier) {
            commands.entity(entity).despawn();
        }
    }
    let Some((at, tier)) = earned else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let missing = hud.missing(relics.iter().map(|(_, _, v, _)| *v));
    if !missing.is_empty() {
        let Some(built) = build(&model.0, tier, &textures, &mut images) else {
            return;
        };
        sounds.play_fx(RELIC_SFX);
        let middle = (built.low + built.high) / 2.0;
        for view in missing {
            let pivot = commands
                .spawn((
                    Relic {
                        tier,
                        height: built.high.y - built.low.y,
                    },
                    HudView(view),
                    Transform::from_scale(Vec3::ZERO),
                    Visibility::default(),
                ))
                .id();
            built.spawn(
                Transform::from_translation(-middle),
                pivot,
                &mut commands,
                &mut meshes,
                &mut materials,
                &mut subtract_materials,
            );
        }
        return;
    }
    let ticks = sim.race.tick.saturating_sub(at) as f32 + fixed.overstep_fraction();
    let grown = (ticks / GROW_TICKS).min(1.0);
    let angle = ticks / SPIN_TICKS * std::f32::consts::TAU;
    for (_, relic, view, mut transform) in &mut relics {
        let Some(kart) = hud.get(view.0) else {
            continue;
        };
        // The HUD camera looks at the middle of the window, with y up.
        let rect = kart.rect(window);
        transform.translation = Vec3::new(
            rect.center().x - window.width() / 2.0,
            window.height() / 2.0 - rect.min.y - RELIC_Y * hud::PIXEL.y,
            -DEPTH,
        );
        transform.scale = Vec3::splat(grown * RELIC_LINES * hud::PIXEL.y / relic.height);
        transform.rotation = Quat::from_rotation_y(angle);
    }
}

/// The relic's mesh, every face in `tier`'s colour.
pub(crate) fn build(
    model: &Model,
    tier: Tier,
    textures: &Textures,
    images: &mut Assets<Image>,
) -> Option<HudModel> {
    let header = model.headers.first()?;
    let frame = crate::props::rest_pose(header)?;
    let mut data = mesh::model_mesh(header, frame, &textures.vram, [0x1000; 3]);
    // (Textured faces: the console doubles their colour, as `model_mesh` does.)
    let color = tint(tier).map(|c| mesh::vertex_channel(c as f32 * 2.0 / 255.0));
    for c in data.parts.iter_mut().flat_map(|p| p.colors.iter_mut()) {
        *c = [color[0], color[1], color[2], 1.0];
    }
    Some(HudModel::new(data, images))
}
