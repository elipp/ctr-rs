//! The end of a solo run: a mockup of retail's relic ceremony (`RR_EndEvent_DrawMenu`, overlay
//! 223) with the same news but none of its layout or timing, shown for a time trial too (less the
//! relic's lines). The time, the perfect bonus, the relic the run earned (the pack's spinning
//! relic in the tier's colour) and the personal best (new or kept), centred on the screen in place
//! of the TIME block, from the settled finish (`CurrentRace::solo_end`) until the player goes on
//! (Cross or Space: not Enter, which opens the command line).

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

/// How long after the finish before Cross or Space goes on: one pressed for the race's last
/// moments shouldn't skip the ceremony.
const CONTINUE_AFTER_TICKS: u32 = crate::GAME_TICK_HZ as u32;

const CONTINUE_KEY: KeyCode = KeyCode::Space;
const CONTINUE: crate::prompt::Prompt =
    crate::prompt::Prompt::new(crate::pad_glyph::PadGlyph::Cross, CONTINUE_KEY, "continue");

/// The prompt goes between yellow and orange this often: slower than the other lines' flash, which
/// would be too loud for a line that stays up.
const PROMPT_BLINK_TICKS: u32 = 8;

/// The relic of the racers' pack.
#[derive(Resource)]
pub struct RelicModel(pub Model);

/// Where the relic's middle is, in lines of retail's screen (the panel's is `Band::SoloEnd`).
const RELIC_Y: f32 = 170.0;

/// Between the panel's lines, in lines of retail's screen.
const ROW_GAP: f32 = 6.0;

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
    Final,
    Relic,
    Best,
    OldBest,
    Submit,
    Continue,
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
            (continue_on_key, panel, relic).distributive_run_if(crate::drive::driving),
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
    last: Res<crate::last_input::LastInput>,
    hud: HudViews,
    panels: Query<(&HudView, &mut Visibility), With<Panel>>,
    mut lines: Query<(&Line, &mut CtrText, &mut Node)>,
) {
    let mut panels = panels;
    for view in hud.missing(panels.iter().map(|(v, _)| *v)) {
        let line = |line: Line| {
            // (The large font has no parentheses.)
            let font = if line == Line::Perfect {
                Font::Middle
            } else {
                Font::Large
            };
            // (An empty line above the prompt.)
            let margin = if line == Line::Continue {
                UiRect::top(hud::y(Font::Large.line_height() as f32 + ROW_GAP))
            } else {
                UiRect::ZERO
            };
            (
                line,
                CtrText::new("", font, Gradient::ORANGE).fixed(hud::PIXEL),
                Node {
                    margin,
                    ..default()
                },
            )
        };
        commands.spawn((
            Panel,
            HudView(view),
            Node {
                row_gap: hud::y(ROW_GAP),
                ..crate::bands::Band::SoloEnd.node()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
            children![
                line(Line::Time),
                line(Line::Perfect),
                line(Line::Final),
                line(Line::Relic),
                line(Line::Best),
                line(Line::OldBest),
                line(Line::Submit),
                line(Line::Continue)
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
    let bonus = if end.perfect { race::PERFECT_BONUS_MS } else { 0 };
    // Retail's text flashes orange and white, a frame each.
    let flash = crate::blink::flash(crate::blink::on_ticks(sim.race.tick, 1));
    for (line, mut text, mut node) in &mut lines {
        let (wanted, gradient) = match line {
            Line::Time => (
                Some(format!("YOUR TIME {}", race::clock::time(total + bonus))),
                Gradient::ORANGE,
            ),
            Line::Perfect => (
                end.perfect
                    .then(|| format!("- {} SECONDS (PERFECT)", bonus / race::clock::SECOND)),
                flash,
            ),
            Line::Final => (
                end.perfect
                    .then(|| format!("FINAL TIME {}", race::clock::time(total))),
                Gradient::ORANGE,
            ),
            Line::Relic => (
                end.relic.map(|tier| format!("{} RELIC AWARDED!", crate::laps::tier_name(tier).to_uppercase())),
                end.relic.map_or(flash, crate::laps::tier_gradient),
            ),
            Line::Best => (end.new_best.then(|| "NEW HIGH SCORE!".to_string()), flash),
            Line::OldBest => (
                end.best.map(|ms| format!("BEST {}", race::clock::time(ms))),
                Gradient::WHITE,
            ),
            Line::Submit => (
                submit_line(end.submit, *last, &config),
                match end.submit {
                    Some(Submit::Offered) => Gradient::YELLOW,
                    Some(Submit::Refused) => Gradient::RED,
                    _ => Gradient::WHITE,
                },
            ),
            Line::Continue => (
                (sim.race.tick >= end.at + CONTINUE_AFTER_TICKS)
                    .then(|| CONTINUE.text(last.pad())),
                if crate::blink::on_ticks(sim.race.tick, PROMPT_BLINK_TICKS) {
                    Gradient::YELLOW
                } else {
                    Gradient::ORANGE
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

/// Cross or Space, once the ceremony has been up a moment, ends the race.
fn continue_on_key(
    sim: Res<crate::props::Sim>,
    mut race_on: ResMut<RaceOn>,
    menu: Res<crate::pause::PauseMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pads: crate::drive::Pads,
) {
    let Some(end) = race_on.get_mut().and_then(|race| race.solo_end.as_mut()) else {
        return;
    };
    if menu.open || end.continued || sim.race.tick < end.at + CONTINUE_AFTER_TICKS {
        return;
    }
    let pressed = keys.just_pressed(CONTINUE_KEY)
        || pads.iter().any(|p| p.just_pressed(GamepadButton::South));
    if !pressed {
        return;
    }
    end.continued = true;
    // (Not a hop in the free drive the race ends into.)
    keys.clear_just_pressed(CONTINUE_KEY);
    pads.consume();
}

/// (The font has no '#'.)
fn submit_line(
    submit: Option<Submit>,
    last: crate::last_input::LastInput,
    config: &crate::config::Config,
) -> Option<String> {
    Some(match submit? {
        Submit::Offered => crate::ghost::select_prompt(config, "submit").text(last.pad()),
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
    between: crate::between::Between,
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
    let ticks = sim.race.tick.saturating_sub(at) as f32 + between.fraction();
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
