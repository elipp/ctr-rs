//! The time and the laps of an online race, as retail draws them. TIME is at the top left: the
//! race's time since the green light, and under it a row for each lap so far ("L1" and its
//! time; the last one is the lap being driven). LAP is at the top right: the lap out of the
//! race's. The times are `m:ss:cc`. The places and colours are measured from retail screenshots.
//!
//! A relic race has no lap rows: under TIME is the next relic to earn and its time instead, and
//! under that the personal best. A time trial has the best between TIME TRIAL and the lap rows.
//! Under the best, a solo run has the board's world record if there is one ("(You)" and white if
//! it is ours). A record that is our best is not shown twice: the best's label says so instead.
//! (The splits against it are toasts, as a race's are: `drive/overlay.rs`.)
//!
//! A free drive has neither: it has no laps to count and no end. In TIME's place it has the
//! track's personal bests, if any, the relic best's time followed by the relic it earned, spinning
//! in its tier's colour as at the end of a relic race (`solo_end`).

use bevy::prelude::*;
use render::font::{Font, Gradient};

use crate::font::CtrText;
use crate::ghost::Source;
use crate::hud;
use crate::hud_view::{HudView, HudViews, ViewKart};
use crate::replay::Watchable;
use crate::subtract::SubtractMaterial;

/// Where the blocks are: both from the top, TIME from the left and LAP from the right, in lines
/// and pixels of retail's screen.
const TOP: f32 = 3.0;
const TIME_LEFT: f32 = 10.0;
const LAP_RIGHT: f32 = 20.0;

/// The lines between a label and what it labels, and between two lap rows.
const LINE_GAP: f32 = 1.0;

/// How far right of the "L1" its time starts, in pixels of retail's screen.
const LAP_TIME_X: f32 = 42.0;

/// "TIME", or "TIME TRIAL" in one (`UI_DrawRaceClock`).
#[derive(Component)]
struct TimeLabel;

/// The space above the lap rows, in lines.
const ROWS_DOWN: f32 = 1.0;

/// Retail's colours of the relics' names and times (`data.colors`): flat ones, but platinum's
/// SILVER, which runs dark to white across a corner, is ours as a top-to-bottom one.
const SAPPHIRE: Gradient = Gradient {
    top: [0x40, 0xbf, 0xff],
    bottom: [0x40, 0xbf, 0xff],
};
const GOLD: Gradient = Gradient {
    top: [0xe6, 0xd3, 0x00],
    bottom: [0xe6, 0xd3, 0x00],
};
const PLATINUM: Gradient = Gradient {
    top: [0xff, 0xff, 0xff],
    bottom: [0x60, 0x60, 0x60],
};

/// A relic's name.
pub(crate) fn tier_name(tier: race::relic::Tier) -> &'static str {
    use race::relic::Tier;
    match tier {
        Tier::Sapphire => "Sapphire",
        Tier::Gold => "Gold",
        Tier::Platinum => "Platinum",
    }
}

/// The colour of a relic's name and time.
pub(crate) fn tier_gradient(tier: race::relic::Tier) -> Gradient {
    use race::relic::Tier;
    match tier {
        Tier::Sapphire => SAPPHIRE,
        Tier::Gold => GOLD,
        Tier::Platinum => PLATINUM,
    }
}

/// The TIME block.
#[derive(Component)]
struct TimeBlock;

/// The LAP block.
#[derive(Component)]
struct LapBlock;

/// The race's time.
#[derive(Component)]
struct Total;

/// The lap out of the race's.
#[derive(Component)]
struct LapCount;

/// The relic to earn's name, and its time.
#[derive(Component)]
struct TargetName;

#[derive(Component)]
struct TargetTime;

/// The personal best's label, and its time.
#[derive(Component)]
struct BestName;

#[derive(Component)]
struct BestTime;

/// The world record's label, and its time.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum World {
    Name,
    Time,
}

/// The column of lap rows, and how many it has.
#[derive(Component)]
struct LapRows(usize);

/// The "L1" of row `.0`, and its time.
#[derive(Component)]
struct RowLabel(usize);

#[derive(Component)]
struct RowTime(usize);

/// A free drive's block of the track's personal bests: the relic run's, then the time trial's.
#[derive(Component)]
struct BestsBlock;

#[derive(Component)]
struct FreeBest(race::solo::Kind, bool);

/// The line of the relic best's time and its relic, which goes out of the layout with the time
/// (an empty line would still take the block's gaps).
#[derive(Component)]
struct FreeRelicLine;

/// The space after the relic best's time that its relic is drawn over.
#[derive(Component)]
struct FreeRelicSlot;

/// The relic over a [`FreeRelicSlot`]: its pivot, the model its child, its middle on the pivot.
#[derive(Component)]
struct FreeRelicIcon {
    tier: race::relic::Tier,
    /// The model's height in world units.
    height: f32,
}

/// How tall the relic after the time is, in lines: a little over the large font's 16.
const ICON_LINES: f32 = 20.0;

/// The slot's width and its gap after the time, in pixels of retail's screen. (The relic is
/// narrower than it is tall, and its arms turn out to its sides.)
const ICON_WIDTH: f32 = 14.0;
const ICON_GAP: f32 = 4.0;

/// How far in front of the HUD camera the relic hangs, in pixels.
const ICON_DEPTH: f32 = 100.0;

/// The personal bests of the track driven freely. `stale` once any race is on: one may have
/// set a best, and the race may have been on this very track.
#[derive(Resource)]
struct FreeBests {
    level: u32,
    stale: bool,
    relic: Option<i32>,
    trial: Option<i32>,
    /// The tier of the relic shown after the relic best's time: None when the block is hidden.
    icon: Option<race::relic::Tier>,
}

impl Default for FreeBests {
    fn default() -> Self {
        FreeBests {
            level: 0,
            stale: true,
            relic: None,
            trial: None,
            icon: None,
        }
    }
}

pub struct LapsPlugin;

impl Plugin for LapsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FreeBests>().add_systems(
            Update,
            (show, (show_bests, free_relic).chain()).distributive_run_if(crate::drive::driving),
        );
    }
}

/// A line of the game's text at the HUD's size.
fn text(text: &str, font: Font, gradient: Gradient) -> CtrText {
    CtrText::new(text, font, gradient).fixed(hud::PIXEL)
}

fn show(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    hud: HudViews,
    race_on: Res<crate::race_on::RaceOn>,
    world_record: Res<crate::world_record::WorldRecord>,
    mut blocks: Query<(&HudView, &mut Visibility), Or<(With<TimeBlock>, With<LapBlock>)>>,
    time_blocks: Query<&HudView, With<TimeBlock>>,
    mut rows: Query<(Entity, &HudView, &mut LapRows)>,
    mut texts: Query<
        (
            &HudView,
            &mut CtrText,
            Has<Total>,
            Has<LapCount>,
            Option<&RowLabel>,
            Option<&RowTime>,
            Has<TargetName>,
            Has<TargetTime>,
            Has<BestName>,
            Has<BestTime>,
            Has<TimeLabel>,
        ),
        Without<World>,
    >,
    mut relic_nodes: Query<
        (&mut Node, Has<BestName>, Has<BestTime>),
        (
            Or<(
                With<TargetName>,
                With<TargetTime>,
                With<BestName>,
                With<BestTime>,
            )>,
            Without<World>,
        ),
    >,
    mut world_texts: Query<(&HudView, &World, &mut CtrText, &mut Node)>,
) {
    let missing = hud.missing(time_blocks.iter().copied());
    if !missing.is_empty() {
        // The first frame, or of a new level (which took the nodes away).
        for view in missing {
            spawn(&mut commands, view);
        }
        return;
    }
    // (The end of a solo run has its own panel, with the time.)
    let shown = if race_on.is_on() && crate::solo_end::shown(&race_on).is_none() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (_, mut visibility) in &mut blocks {
        visibility.set_if_neq(shown);
    }
    if !race_on.is_on() {
        return;
    }
    let race = &*sim.race;
    let solo = race_on.get().filter(|race| race.solo.is_some());
    let relic = solo.filter(|race| race.solo == Some(race::solo::Kind::Relic));
    let trial = solo.is_some() && relic.is_none();
    let target = relic.and_then(|race| race.relic_target).map(|(tier, ms)| {
        use race::relic::Tier;
        match tier {
            Tier::Sapphire => ("Sapphire", SAPPHIRE, ms),
            Tier::Gold => ("Gold", GOLD, ms),
            Tier::Platinum => ("Platinum", PLATINUM, ms),
        }
    });
    let best = solo.and_then(|race| race.best);
    let world = solo.and_then(|race| world_record.of(race));
    let world_is_best = solo.is_some_and(|race| world_record.is_best(race));
    // (Out of the layout without one, or the block's gaps would push the lap rows down; the best
    // goes up in the target's place.)
    let display = |on: bool| if on { Display::Flex } else { Display::None };
    for (mut node, best_name, best_time) in &mut relic_nodes {
        let display = display(if best_name || best_time {
            best.is_some()
        } else {
            target.is_some()
        });
        if node.display != display {
            node.display = display;
        }
    }
    for ViewKart {
        view, slot, driver, ..
    } in hud.iter()
    {
        let finished = driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
        let done = &race.lap_times[slot];
        // The time stops when we cross the line of the last lap: then it is the laps' times (a
        // perfect relic run's less its bonus).
        let total = race.final_ms(slot).unwrap_or(race.event_ms);
        // A kart that finished stays on its last lap.
        let laps = race.mode.lap_limit();
        let lap = (driver.lap_index + 1).min(laps);
        // The laps done, and the one being driven (the first counts from the green light).
        let mut times: Vec<i32> = done.clone();
        if relic.is_some() {
            times.clear();
        } else if !finished {
            times.push(race.event_ms - driver.lap_time);
        }
        for (v, &part, mut text, mut node) in &mut world_texts {
            if v.0 != view {
                continue;
            }
            let wanted = match part {
                World::Name | World::Time if world_is_best => None,
                World::Name => world.map(|(_, _, ours)| {
                    if ours {
                        "Server Record (You)"
                    } else {
                        "Server Record"
                    }
                    .into()
                }),
                World::Time => world.map(|(entry, _, _)| race::clock::time(entry.final_ms)),
            };
            if let Some((_, _, ours)) = world {
                let gradient = if ours {
                    Gradient::WHITE
                } else {
                    Gradient::YELLOW
                };
                if text.gradient != gradient {
                    text.gradient = gradient;
                }
            }
            let display = display(wanted.is_some());
            if node.display != display {
                node.display = display;
            }
            if let Some(wanted) = wanted
                && text.text != wanted
            {
                text.text = wanted;
            }
        }
        if let Some((entity, _, mut count)) = rows.iter_mut().find(|(_, v, _)| v.0 == view)
            && count.0 != times.len()
        {
            count.0 = times.len();
            commands.entity(entity).despawn_related::<Children>();
            for (i, &ms) in times.iter().enumerate() {
                spawn_row(&mut commands, entity, view, i, &race::clock::time(ms));
            }
        }
        for (
            v,
            mut text,
            is_total,
            is_count,
            label,
            time,
            is_name,
            is_target,
            is_best_name,
            is_best,
            is_label,
        ) in &mut texts
        {
            if v.0 != view {
                continue;
            }
            if (is_name || is_target)
                && let Some((_, gradient, _)) = target
                && text.gradient != gradient
            {
                text.gradient = gradient;
            }
            let wanted = if is_label {
                if trial { "Time Trial" } else { "Time" }.to_string()
            } else if is_total {
                race::clock::time(total)
            } else if is_count {
                format!("{lap}/{laps}")
            } else if let Some(RowLabel(i)) = label {
                format!("L{}", i + 1)
            } else if let Some(&RowTime(i)) = time {
                times.get(i).map_or_else(String::new, |&ms| race::clock::time(ms))
            } else if is_name {
                target.map_or_else(String::new, |(name, _, _)| name.to_string())
            } else if is_target {
                target.map_or_else(String::new, |(_, _, ms)| race::clock::time(ms))
            } else if is_best_name {
                if world_is_best {
                    "Personal Best (Server Record)"
                } else {
                    "Personal Best"
                }
                .to_string()
            } else if is_best {
                best.map_or_else(String::new, race::clock::time)
            } else {
                continue;
            };
            if text.text != wanted {
                text.text = wanted;
            }
        }
    }
}

/// A replay is only needed when a best's `.time` is missing or stale (`pb.rs`): otherwise this
/// reads two small files per track.
fn show_bests(
    race_on: Res<crate::race_on::RaceOn>,
    lobby: Option<Res<crate::lobby::Lobby>>,
    level: Res<crate::resources::LevelNumber>,
    mut bests: ResMut<FreeBests>,
    mut blocks: Query<&mut Visibility, With<BestsBlock>>,
    mut texts: Query<(&FreeBest, &mut CtrText, &mut Node)>,
    mut relic_line: Query<&mut Node, (With<FreeRelicLine>, Without<FreeBest>)>,
) {
    // (A lobby's free drive has its own things up there.)
    if race_on.is_on() || lobby.is_some_and(|lobby| lobby.is_open()) {
        bests.stale = true;
        bests.icon = None;
        for mut visibility in &mut blocks {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    if bests.stale || bests.level != level.0 {
        let laps = race::DEFAULT_LAPS;
        let best = |kind| crate::pb::best_on(level.0, kind, laps).map(|o| o.final_ms);
        *bests = FreeBests {
            level: level.0,
            stale: false,
            relic: best(race::solo::Kind::Relic),
            trial: best(race::solo::Kind::TimeTrial),
            icon: None,
        };
    }
    let shown = if bests.relic.is_some() || bests.trial.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut blocks {
        visibility.set_if_neq(shown);
    }
    let tier = bests
        .relic
        .and_then(|ms| race::relic::earned(level.0, race::DEFAULT_LAPS, ms));
    let icon = tier.filter(|_| shown == Visibility::Inherited);
    if bests.icon != icon {
        bests.icon = icon;
    }
    for mut node in &mut relic_line {
        let display = if bests.relic.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
    for (&FreeBest(kind, is_time), mut text, mut node) in &mut texts {
        let ms = match kind {
            race::solo::Kind::Relic => bests.relic,
            race::solo::Kind::TimeTrial => bests.trial,
        };
        let display = if ms.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        let wanted = if is_time {
            ms.map_or_else(String::new, race::clock::time)
        } else {
            continue;
        };
        if text.text != wanted {
            text.text = wanted;
        }
    }
}

/// Puts the relic after the relic best's time when it earned one, in its tier's colour, and
/// turns it as the ceremony's turns.
#[allow(clippy::too_many_arguments)]
fn free_relic(
    mut commands: Commands,
    sim: Res<crate::props::Sim>,
    between: crate::between::Between,
    bests: Res<FreeBests>,
    model: Res<crate::solo_end::RelicModel>,
    textures: Res<crate::Textures>,
    hud: HudViews,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    slots: Query<(&HudView, &ComputedNode, &UiGlobalTransform), With<FreeRelicSlot>>,
    mut icons: Query<(Entity, &FreeRelicIcon, &HudView, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
) {
    for (entity, icon, ..) in &icons {
        if bests.icon != Some(icon.tier) {
            commands.entity(entity).despawn();
        }
    }
    let Some(tier) = bests.icon else {
        return;
    };
    let missing = hud.missing(
        icons
            .iter()
            .filter(|(_, icon, ..)| icon.tier == tier)
            .map(|(_, _, v, _)| *v),
    );
    if !missing.is_empty() {
        let Some(built) = crate::solo_end::build(&model.0, tier, &textures, &mut images) else {
            return;
        };
        let middle = (built.low + built.high) / 2.0;
        for view in missing {
            let pivot = commands
                .spawn((
                    FreeRelicIcon {
                        tier,
                        height: built.high.y - built.low.y,
                    },
                    HudView(view),
                    // (Out of sight until placed over its slot.)
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
    let Ok(window) = windows.single() else {
        return;
    };
    let ticks = sim.race.tick as f32 + between.fraction();
    let angle = ticks / crate::solo_end::SPIN_TICKS * std::f32::consts::TAU;
    for (_, icon, view, mut transform) in &mut icons {
        let (Some(kart), Some((_, slot, at))) =
            (hud.get(view.0), slots.iter().find(|(v, ..)| *v == view))
        else {
            continue;
        };
        // Laid out in the view's part of the window, in its physical pixels; the HUD camera looks
        // at the middle of the whole window, with y up.
        let centre = kart.rect(window).min + at.translation * slot.inverse_scale_factor;
        transform.translation = Vec3::new(
            centre.x - window.width() / 2.0,
            window.height() / 2.0 - centre.y,
            -ICON_DEPTH,
        );
        transform.scale = Vec3::splat(ICON_LINES * hud::PIXEL.y / icon.height);
        transform.rotation = Quat::from_rotation_y(angle);
    }
}

/// Both blocks of local player `view`, and the free drive's bests, hidden, with no lap rows.
fn spawn(commands: &mut Commands, view: usize) {
    let block = |left: Option<f32>, right: Option<f32>, align| Node {
        position_type: PositionType::Absolute,
        left: left.map_or(Val::Auto, hud::x),
        right: right.map_or(Val::Auto, hud::x),
        top: hud::y(TOP),
        flex_direction: FlexDirection::Column,
        align_items: align,
        row_gap: hud::y(LINE_GAP),
        ..default()
    };
    let view = HudView(view);
    commands.spawn((
        TimeBlock,
        view,
        block(Some(TIME_LEFT), None, AlignItems::FlexStart),
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            (
                view,
                TimeLabel,
                text("Time", Font::Middle, Gradient::ORANGE)
            ),
            (view, Total, text("0:00:00", Font::Large, Gradient::ORANGE)),
            (view, TargetName, text("", Font::Middle, SAPPHIRE)),
            (view, TargetTime, text("", Font::Large, SAPPHIRE)),
            (
                view,
                BestName,
                Watchable(Source::Best),
                text("Personal Best", Font::Middle, Gradient::WHITE)
            ),
            (
                view,
                BestTime,
                Watchable(Source::Best),
                text("", Font::Large, Gradient::WHITE)
            ),
            (
                view,
                World::Name,
                Watchable(Source::World),
                text("", Font::Middle, Gradient::YELLOW)
            ),
            (
                view,
                World::Time,
                Watchable(Source::World),
                text("", Font::Large, Gradient::YELLOW)
            ),
            (
                view,
                LapRows(0),
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: hud::y(LINE_GAP),
                    margin: UiRect::top(hud::y(ROWS_DOWN)),
                    ..default()
                },
            ),
        ],
    ));
    use race::solo::Kind;
    commands.spawn((
        BestsBlock,
        view,
        block(Some(TIME_LEFT), None, AlignItems::FlexStart),
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            (
                text("Personal Bests", Font::Middle, Gradient::WHITE),
                Node {
                    margin: UiRect::bottom(hud::y(LINE_GAP)),
                    ..default()
                },
            ),
            (
                FreeBest(Kind::Relic, false),
                text("Relic", Font::Middle, Gradient::ORANGE)
            ),
            (
                FreeRelicLine,
                Node {
                    align_items: AlignItems::Center,
                    column_gap: hud::x(ICON_GAP),
                    ..default()
                },
                children![
                    (
                        FreeBest(Kind::Relic, true),
                        text("", Font::Large, Gradient::ORANGE)
                    ),
                    (
                        view,
                        FreeRelicSlot,
                        Node {
                            width: hud::x(ICON_WIDTH),
                            height: hud::y(Font::Large.line_height() as f32),
                            ..default()
                        },
                    ),
                ],
            ),
            (
                FreeBest(Kind::TimeTrial, false),
                text("Time Trial", Font::Middle, Gradient::ORANGE)
            ),
            (
                FreeBest(Kind::TimeTrial, true),
                text("", Font::Large, Gradient::ORANGE)
            ),
        ],
    ));
    commands.spawn((
        LapBlock,
        view,
        block(None, Some(LAP_RIGHT), AlignItems::FlexEnd),
        Visibility::Hidden,
        Pickable::IGNORE,
        children![
            text("Lap", Font::Middle, Gradient::WHITE),
            (view, LapCount, text("", Font::Large, Gradient::WHITE)),
        ],
    ));
}

/// Row `i` of the lap rows: the red "L1" and the white time after it.
fn spawn_row(commands: &mut Commands, rows: Entity, view: usize, i: usize, time: &str) {
    commands.spawn((
        Node::default(),
        ChildOf(rows),
        children![
            (
                HudView(view),
                RowLabel(i),
                text(&format!("L{}", i + 1), Font::Middle, Gradient::RED),
                // The time starts at the same place in every row.
                Node {
                    min_width: hud::x(LAP_TIME_X),
                    ..default()
                },
            ),
            (
                HudView(view),
                RowTime(i),
                text(time, Font::Middle, Gradient::WHITE)
            ),
        ],
    ));
}
