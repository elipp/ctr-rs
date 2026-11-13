//! One game tick of the race: the pads of the others, rollback, and what follows from it.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use race::Pad;
use race::rollback::Step;
use vehicle::{Effect, buttons};

use super::input::{Controller, pad_of};
use super::local::Follow;
use super::overlay::NetHud;
use super::pose::{Pose, pose_of};
use super::{Drive, Kart};
use crate::Track;
use crate::command::Action;
use crate::kart::{KartMotion, KartSlot};
use crate::toast::Toast;

/// How many ticks we may play past the last pad of another player before we wait for it: the
/// prediction is that long at most. The clock is kept with the others' without this (see
/// `lobby::pace`), so it only matters when one of them stalls for good: 2 seconds.
const MAX_LEAD: u32 = 60;

/// How fast a spectator's count of the ticks in hand follows the count of each tick.
const BUFFERED_SMOOTHING: f32 = 0.1;

/// How often the toast "Waiting for ..." is put up again while we wait: every 3 seconds, and it
/// stays that long.
const WAIT_TOAST_TICKS: u32 = 90;

/// Freezes this client for `time` (`/debug stall`). A browser cannot sleep: there it spins.
fn stall(time: std::time::Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(time);
    #[cfg(target_arch = "wasm32")]
    {
        let until = net::Instant::now() + time;
        while net::Instant::now() < until {
            std::hint::spin_loop();
        }
    }
}

/// What a tick shows of itself: the particles, the karts' drawn motion, the labels, toasts and
/// sounds.
#[derive(SystemParam)]
pub(super) struct Shown<'w, 's> {
    particles: ResMut<'w, crate::exhaust::Particles>,
    motions: Query<'w, 's, (&'static mut KartMotion, &'static mut KartSlot, Has<Kart>)>,
    smoothing: Res<'w, crate::kart::Smoothing>,
    net_label: Query<'w, 's, &'static mut Text, With<NetHud>>,
    toasts: MessageWriter<'w, Toast>,
    sfx: crate::sound::Sfx<'w>,
}

/// What [`drive_tick`] keeps from one tick to the next.
#[derive(Default)]
pub(super) struct Memory {
    was_paused: bool,
    /// The ticks waited for a late player, and the buttons held last tick (a watching client's
    /// pick the kart to follow).
    waits: (u32, u32),
}

/// One game tick of the sim.
pub(super) fn drive_tick(
    controller: Controller,
    track: Res<Track>,
    mut sim: ResMut<crate::props::Sim>,
    mut drive: ResMut<Drive>,
    mut hold: crate::fx::Hold,
    mut lobby: ResMut<crate::lobby::Lobby>,
    config: Res<crate::config::Config>,
    shown: Shown,
    mut memory: Local<Memory>,
    mut brains: ResMut<crate::learned::Brains>,
) {
    let Shown {
        mut particles,
        mut motions,
        smoothing,
        mut net_label,
        mut toasts,
        mut sfx,
    } = shown;
    let Memory { was_paused, waits } = &mut *memory;
    let drive = &mut *drive;
    // In an online race the sim is everyone's: no pause, no restarts.
    let online = lobby.online.is_some();
    // The level of the race is not up yet, nor the karts of the others.
    if lobby
        .online
        .as_ref()
        .is_some_and(|net| net.track != track.1.0 || !net.built)
    {
        return;
    }

    // The pause menu (or the effects panel's freeze) stops the race, unless the race is
    // everyone's.
    let paused = (hold.menu.open || hold.fx.holds()) && !online;
    if paused && !*was_paused {
        // Nothing moves between ticks now, so the interpolation must not either.
        for view in &mut drive.views {
            view.freeze();
        }
        for (mut motion, _, _) in &mut motions {
            motion.freeze();
        }
    }
    *was_paused = paused;
    if paused {
        return;
    }

    // The menu's RESTART is the grid: every start puts everything, the objects too, back
    // as it was at tick 0.
    let picked = std::mem::take(&mut hold.menu.restart);
    let mut restart = picked && !online;
    // The lobby closed, or the race failed: back to the grid with our kart alone.
    if !online && sim.race.karts.len() != drive.views.len() {
        restart = true;
    }
    // The race starts: tick 0 for everyone.
    if let Some(net) = lobby.online.as_mut().filter(|net| net.fresh) {
        net.fresh = false;
        restart = true;
    }
    if restart {
        let (specs, own, spectator) = match &lobby.online {
            Some(net) => (net.specs.clone(), net.viewed(), net.spectating()),
            None => (drive.specs(), 0, false),
        };
        let bots = lobby.online.as_ref().map(|net| {
            let setup = vehicle::bot::Setup {
                difficulty: net.retail.difficulty as i16,
                // A custom track has the rubber band of the first.
                track: (net.track < race::RACING_TRACKS).then_some(net.track as u8),
                cup: net.retail.cup,
                players: net.characters.len(),
            };
            (setup, net.retail_specs.as_slice())
        });
        // The host's brain bots are players to the race: it rubber-bands them (at the difficulty
        // of `/bots`, which comes with the retail bots' even when there are none).
        let band = lobby
            .online
            .as_ref()
            .filter(|net| net.bots > 0)
            .map(|net| race::Band::new(net.bot_slots(), net.retail.difficulty as i16));
        sim.restart(
            &track.0,
            &specs,
            own,
            lobby.online.as_ref().map(|net| net.laps),
            spectator,
            bots,
            band,
        );
        // Online, the host's bots are the policy's too: their pads are made here, and go to the
        // others as ours do.
        let hosted = lobby.online.as_ref().filter(|net| net.hosts_bots());
        if hosted.is_some() {
            match brains.load() {
                Ok(name) => info!("online bots: the policy {name}"),
                // (They hold a neutral pad then, below: the race does not wait for them.)
                Err(e) => {
                    error!("online bots: {e}");
                    toasts.write(Toast::new(e, 4000).color(Color::srgb(1.0, 0.4, 0.4)));
                }
            }
        }
        // (Also with none: the karts of the race before are not driven any more. Our own, if
        // `/autopilot` is on, is taken again below.)
        brains.start(
            &track.0,
            track.1,
            &sim.race,
            &specs,
            hosted.map_or(0..0, |net| net.bot_slots()),
            hosted.map_or(vehicle::bot::difficulty_level::MEDIUM, |net| {
                net.retail.difficulty as i16
            }),
        );
        particles.list.clear();
        for view in &mut drive.views {
            view.reset(&track);
        }
        drive.own().slot = own;
        each_motion(&sim, &mut motions, |motion, kart| motion.reset(kart));
    }

    // Tick 0 has to come first: the others must be ready as well.
    if lobby.online.as_ref().is_some_and(|net| !net.go) {
        for mut text in &mut net_label {
            text.0 = match &lobby.online {
                Some(net) if config.debug_hud => net.summary(0, 0),
                _ => String::new(),
            };
        }
        return;
    }

    let tick = sim.race.tick();
    let own = sim.own;
    let spectating = lobby.online.as_ref().is_some_and(|net| net.spectating());
    let mut reports = Vec::new();
    if let Some(net) = lobby.online.as_mut() {
        // The pads of the others that came in since the last tick.
        for (slot, first, pads) in net.inbox.drain(..) {
            let pads: Vec<Pad> = pads.iter().map(pad_of).collect();
            if let Err(hole) = sim.race.confirm_all(slot, first, &pads) {
                error!(
                    "pads of player {slot} from tick {first}: missing from {}",
                    hole.expected
                );
            }
        }
        // A player left: say what we hold of its pads, which is all we take from now on.
        for slot in std::mem::take(&mut net.asked) {
            if slot < sim.race.players() {
                net.frozen[slot] = true;
                let (first, pads) = sim.race.recent(slot);
                reports.push((
                    slot,
                    first,
                    pads.iter()
                        .map(|p| net::Pad {
                            held: p.held,
                            lx: p.lx,
                            ly: p.ly,
                            ry: p.ry,
                        })
                        .collect::<Vec<_>>(),
                ));
            } else {
                // A spectator left: it made no pads, and the host waits for our word all the same.
                reports.push((slot, 0, Vec::new()));
            }
        }
        // The host settled where the pads of a player that left end: everyone ends them there.
        for (slot, first, pads) in std::mem::take(&mut net.lefts) {
            if slot >= sim.race.players() {
                continue;
            }
            let pads: Vec<Pad> = pads.iter().map(pad_of).collect();
            if let Err(hole) = sim.race.confirm_all(slot, first, &pads) {
                // More than the pads kept were missing.
                error!(
                    "the pads of player {slot} that left: missing from {}",
                    hole.expected
                );
                net.found([race::desync::Mismatch {
                    tick: hole.expected,
                    slot,
                    hashes: None,
                }]);
            }
            sim.race.leave(slot);
            net.frozen[slot] = true;
            net.gone[slot] = true;
            let name = net.labels.get(slot).map_or("?", String::as_str);
            toasts.write(
                Toast::new(format!("Player {name} left the race"), 3000)
                    .color(Color::srgb(1.0, 0.6, 0.3)),
            );
        }
    }
    for (slot, first, pads) in reports {
        lobby.have(slot, first, pads);
    }
    // Who has crossed the line, as the sim has it now. The sim takes no more of their pads, so
    // once ours has, we watch the rest of the race as a spectator does (below).
    let watches = match lobby.online.as_mut() {
        Some(net) => {
            net.finished = sim
                .race
                .karts
                .iter()
                .map(|kart| kart.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0)
                .collect();
            net.watches()
        }
        None => spectating,
    };
    // Each client runs its own 30 Hz clock, so one that is ahead would stay ahead (up to the
    // rollback window) and predict the others further and further. It waits for them instead.
    // A spectator has nothing to show quickly, so it predicts nothing: it plays a tick once
    // everyone's pad for it is in, and its karts are drawn as they really went (a hop that is
    // over before its pad comes would not show otherwise). `pace` keeps a few such ticks in
    // hand, so that it seldom waits.
    // A player that has finished does the same: its kart takes no pad any more, so there is
    // nothing to be quick about. When it crosses the line its sim is ahead of the ticks that
    // have everyone's pad, and it waits for those to catch up: the race stops for about the
    // others' delay. A watching client does not wait for the karts that have finished: the sim
    // ignores their pads (and they would wait for each other).
    let finished_slot = |slot: usize| {
        lobby
            .online
            .as_ref()
            .is_some_and(|net| net.finished.get(slot).copied().unwrap_or(false))
    };
    let ours = |slot: usize| lobby.online.as_ref().is_some_and(|net| net.drives(slot));
    let late: Vec<usize> = if online {
        (0..sim.race.players())
            .filter(|&slot| spectating || slot != own)
            // (Our bots' pads are made below.)
            .filter(|&slot| !ours(slot))
            .filter(|&slot| !(watches && finished_slot(slot)))
            .filter(|&slot| {
                let until = sim.race.confirmed_until(slot);
                if watches {
                    tick >= until
                } else {
                    tick > until.saturating_add(MAX_LEAD)
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    if watches && let Some(net) = lobby.online.as_mut() {
        // (Once everyone has left or finished, nothing is waited for: as if the buffer were
        // full.)
        let complete = (0..sim.race.players())
            .filter(|&slot| !net.finished[slot])
            .map(|slot| sim.race.confirmed_until(slot))
            .min()
            .unwrap_or(u32::MAX);
        let buffered = if complete == u32::MAX {
            crate::lobby::SPECTATE_BUFFER
        } else {
            complete.saturating_sub(tick) as f32
        };
        net.buffered += (buffered - net.buffered) * BUFFERED_SMOOTHING;
    }
    if !late.is_empty() {
        // (The host removes a player that stays silent: see `net::KICK_AFTER`.) A spectator
        // (or a finished player) waits a tick now and then; it says so when it is as late as a
        // player would wait.
        let quiet = if watches { MAX_LEAD } else { 0 };
        if waits.0 >= quiet
            && (waits.0 - quiet) % WAIT_TOAST_TICKS == 0
            && let Some(net) = lobby.online.as_ref()
        {
            let names: Vec<&str> = late
                .iter()
                .map(|&slot| net.labels.get(slot).map_or("?", String::as_str))
                .collect();
            let text = format!("Waiting for {}...", names.join(", "));
            toasts.write(
                Toast::new(
                    text,
                    WAIT_TOAST_TICKS as u64 * 1000 / crate::GAME_TICK_HZ as u64,
                )
                .color(Color::srgb(1.0, 0.8, 0.2)),
            );
        }
        waits.0 += 1;
        return;
    }
    waits.0 = 0;
    // The pads that are one player's own: the others do not read them.
    let taken: Vec<Entity> = drive.views.iter().filter_map(|v| v.device.pad()).collect();
    // Which way R1 and L1 were tapped this tick, to change the kart the camera follows.
    let mut cycle = 0;
    // A spectator has no pad to play, nor does a finished player that the sim ignores, but their
    // R1 and L1 pick the kart to follow all the same.
    if watches {
        let held = controller.read(&drive.views[0].device, &taken, waits.1).buttons_held;
        let taps = held & !waits.1;
        waits.1 = held;
        cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
    }
    // Online, the host's bots: from the race as it is now, the others' pads predicted, as a
    // player sees it. They go to the others before our own pad, in the same packet.
    // Our own kart too, if `/autopilot` is on, made by the same policy. Not once we watch: a
    // spectator has no kart, and a finished one takes no pad.
    let auto = brains.autopilot && !spectating && !watches;
    brains.drive_own(&track.0, track.1, &sim.race, own, auto);
    let own_due = auto && sim.race.confirmed_until(own) <= tick;
    let bots_due = lobby
        .online
        .as_ref()
        .is_some_and(|net| net.hosts_bots() && net.bot_slots().any(|slot| sim.race.confirmed_until(slot) <= tick));
    let mut pads = if bots_due || own_due {
        brains.pads(&track.0, &sim.race)
    } else {
        Vec::new()
    };
    // (Ours goes below, with the controller's taps.)
    let auto_pad = pads.iter().find(|(slot, _)| *slot == own && own_due).map(|&(_, pad)| pad);
    pads.retain(|(slot, _)| *slot != own || !own_due);
    if bots_due
        && let Some(net) = lobby.online.as_ref()
    {
        // No policy (it could not be read): they hold still, as a player that left does.
        if pads.is_empty() {
            pads = net.bot_slots().map(|slot| (slot, Pad::default())).collect();
        }
        for &(slot, pad) in &pads {
            sim.race
                .confirm(slot, tick, pad)
                .expect("the bots' pads come in order");
        }
        lobby.send_bots(
            pads.iter()
                .map(|(_, pad)| net::Pad {
                    held: pad.held,
                    lx: pad.lx,
                    ly: pad.ly,
                    ry: pad.ry,
                })
                .collect(),
        );
    }
    // Our own pad for this tick, from the controller as it is now. It is played at once, and the
    // others get it as well. (A tick that waits plays the pad it already has.) Once we have
    // finished it is neutral: the sim ignores it, and the others still wait for it (so that the
    // host does not take us for gone), and predict it right.
    if !spectating && sim.race.confirmed_until(own) <= tick {
        let held = sim.kart().held;
        let pad = if watches {
            Pad::default()
        } else if online && hold.menu.open {
            // The menu has the controls: the kart coasts.
            Pad::default()
        } else if let Some(pad) = auto_pad {
            pad
        } else {
            Pad::of_input(&controller.read(&drive.views[0].device, &taken, held))
        };
        if !watches {
            // (The taps are the controller's even when the policy drives: its pad is not ours.)
            let buttons_held = match auto_pad {
                Some(_) => controller
                    .read(&drive.views[0].device, &taken, waits.1)
                    .buttons_held,
                None => pad.held,
            };
            let taps = buttons_held & !if auto_pad.is_some() { waits.1 } else { held };
            cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
            // (So that an R1 held over the line is not taken for a tap once we watch.)
            waits.1 = buttons_held;
        }
        sim.race
            .confirm(own, tick, pad)
            .expect("our pads come in order");
        if online {
            lobby.send(net::Pad {
                held: pad.held,
                lx: pad.lx,
                ly: pad.ly,
                ry: pad.ry,
            });
        }
    }
    // The other local players' pads, from their own devices. (Their karts are in the race
    // already, and the pads are in before the tick is played, as ours is.)
    for view in drive.views.iter().skip(1) {
        if sim.race.confirmed_until(view.slot) > tick {
            continue;
        }
        let held = sim.race.karts[view.slot].held;
        let pad = Pad::of_input(&controller.read(&view.device, &taken, held));
        sim.race
            .confirm(view.slot, tick, pad)
            .expect("their pads come in order");
    }
    for mut text in &mut net_label {
        text.0 = match &lobby.online {
            Some(net) if config.debug_hud => net.summary(tick, sim.race.unsettled()),
            _ => String::new(),
        };
    }

    // Everyone's karts play together, by slot, ours among them. The pads that are not in yet
    // are predicted, and put right when they come.
    // A late pad puts the karts of the others right, which would show as a jump: the drawing
    // eases from where the kart was predicted to be to where it is now.
    let mut replayed = 0;
    if sim.race.dirty() {
        let predicted: Vec<Pose> = sim.race.karts.iter().map(|k| pose_of(&k.driver)).collect();
        replayed = sim.race.correct(&track.0);
        for (mut motion, slot, ours) in &mut motions {
            if let (false, Some(before), Some(kart)) =
                (ours, predicted.get(slot.0), sim.race.karts.get(slot.0))
            {
                motion.absorb(before, kart, replayed);
            }
        }
    }
    let advance = sim.race.advance(&track.0);
    replayed += advance.replayed;
    if replayed > 0
        && let Some(net) = lobby.online.as_mut()
    {
        net.corrections += 1;
        net.replayed += replayed;
    }
    // The hashes of the ticks that are final now: the others get ours, and ours are held up
    // against theirs.
    for (tick, hash) in sim.race.take_checksums() {
        lobby.checksum(tick, hash);
        if let Some(net) = lobby.online.as_mut() {
            let found = net.checks.mine(tick, hash);
            net.found(found);
        }
    }
    let Step::Played(ticks) = advance.step else {
        return;
    };
    let mine = &ticks[sim.own];
    // Everyone's exhaust and sparks, ours and the others' (the others' as the sim has them).
    for (slot, kart) in ticks.iter().enumerate() {
        particles.sparks(slot, kart.landing, kart.wall);
        particles.spray(slot, &kart.spray, tick);
        particles.puffs(slot, &kart.puffs);
        if let Some(racer) = sim.race.karts.get(slot) {
            particles.splashes(slot, racer.driver.wake_particles);
            particles.clods(slot, racer.driver.mud_particles);
        }
    }
    let heights: Vec<i32> = sim
        .race
        .karts
        .iter()
        .map(|k| k.driver.inst_pos[1])
        .collect();
    particles.tick(&heights);

    // The mask grab put the kart back on the track: the camera swings round behind it.
    if mine.result.respawned {
        drive.own().camera.flags |= camera::flags::DIRECTION_CHANGED;
    }

    // A boost pulls the camera back.
    if mine
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CameraFireSpeedZoom { .. }))
    {
        drive.own().camera.flags |= camera::flags::FIRE_SPEED_ZOOM;
    }
    // Once we have finished, the camera can follow the others (R1 and L1), the ones still here.
    let finished = sim.driver().actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
    if let Some(net) = lobby.online.as_mut() {
        let players = sim.race.players();
        let before = net.watching;
        // The next kart that way that is still here.
        let step = |net: &mut crate::lobby::Online, step: i32| {
            for _ in 0..players {
                net.watching = (net.watching as i32 + step).rem_euclid(players as i32) as usize;
                if !net.gone[net.watching] {
                    break;
                }
            }
        };
        if spectating {
            // Any kart of the race that is still here, whenever.
            if cycle != 0 || net.gone[net.watching] {
                step(net, if cycle == 0 { 1 } else { cycle });
            }
        } else {
            // After our finish, R1 goes to the kart one place behind the watched one in the race
            // and L1 to the one ahead (wrapping round), the retail bots' included.
            let karts = sim.race.karts.len();
            let here = |net: &crate::lobby::Online, slot: usize| !net.gone.get(slot).copied().unwrap_or(false);
            if !finished || net.watching >= karts || !here(net, net.watching) {
                net.watching = own;
            }
            if finished && cycle != 0 {
                let mut by_rank: Vec<(i16, usize)> = (0..karts)
                    .filter(|&slot| here(net, slot))
                    .map(|slot| (sim.race.karts[slot].driver.driver_rank, slot))
                    .collect();
                by_rank.sort();
                if let Some(at) = by_rank.iter().position(|&(_, slot)| slot == net.watching) {
                    let next = (at as i32 + cycle).rem_euclid(by_rank.len() as i32) as usize;
                    net.watching = by_rank[next].1;
                }
            }
        }
        if net.watching != before {
            drive.own().camera.flags |= camera::flags::DIRECTION_CHANGED;
        }
    }
    let watching = lobby
        .online
        .as_ref()
        .map_or(own, |net| net.watching);
    // The HUD and the sounds follow the kart the spectator watches, as they follow ours.
    if spectating {
        sim.own = watching;
    }
    let ours_finished = !spectating && finished;
    let ours = &sim.race.karts[sim.own].driver;
    let won = ours.actions_flag_set & vehicle::actions::RACE_FINISHED != 0 && ours.driver_rank == 0;
    drive.own().follow(
        &track,
        Follow {
            tick,
            kart: &sim.race.karts[watching],
            intro: sim.race.intro_ticks > 0,
            finished: ours_finished,
            cycle: cycle != 0,
            elsewhere: watching != own,
            confetti: hold.fx.confetti.apply(won),
        },
    );

    // The offsets of the others' karts fade as far as their players' pings call for.
    for (mut motion, slot, ours) in &mut motions {
        if !ours {
            let ping = lobby
                .online
                .as_ref()
                .and_then(|net| net.names.get(slot.0))
                .and_then(|name| lobby.average_pings.get(name));
            let ease = smoothing.ease(ping.copied(), motion.replayed_average());
            motion.set_ease(ease);
        }
    }
    each_motion(&sim, &mut motions, |motion, kart| motion.push(kart));
    // The other local players' views: their karts are never watched or finished-for-the-camera.
    for view in drive.views.iter_mut().skip(1) {
        let kart = &sim.race.karts[view.slot];
        let finished = kart.driver.actions_flag_set & vehicle::actions::RACE_FINISHED != 0;
        view.follow(
            &track,
            Follow {
                tick,
                kart,
                intro: sim.race.intro_ticks > 0,
                finished,
                cycle: false,
                elsewhere: false,
                confetti: hold
                    .fx
                    .confetti
                    .apply(finished && kart.driver.driver_rank == 0),
            },
        );
        view.push();
    }
    drive.own().push();

    // The tick's sounds, the other karts' heard from where the camera is now.
    let places: Vec<Vec3> = sim
        .race
        .karts
        .iter()
        .map(|kart| pose_of(&kart.driver).translation)
        .collect();
    let ear = drive.own().current_view.transform();
    sfx.tick(tick, &sim.race, &track.0, &ticks, sim.own, &places, &ear);
}

/// The debug commands that change our kart: `/debug set weapon <N>` drops what we hold and rolls
/// item `N`, as the roulette would (5, the spring, turns into a turbo), and `/debug set wumpa <N>`
/// sets the wumpa count. Not online.
pub(super) fn debug_actions(
    mut actions: MessageReader<Action>,
    mut sim: ResMut<crate::props::Sim>,
    lobby: Res<crate::lobby::Lobby>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        // Freeze for as long as asked, to see the race put right after a hitch: the frame is cut
        // short, so the others run ahead.
        if let Action::DebugStall(time) = action {
            if lobby.online.is_some() {
                debug!("(debug) stalling for {time:?}");
                stall(*time);
            } else {
                toasts.write(Toast::new("Only online", 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            }
            continue;
        }
        if !matches!(
            action,
            Action::DebugAddWeapon(_)
                | Action::DebugSetWumpa(_)
                | Action::DebugFx(crate::fx::Fx::Finish)
        ) {
            continue;
        }
        if lobby.online.is_some() {
            toasts.write(Toast::new("Not online", 2500).color(Color::srgb(1.0, 0.4, 0.4)));
            continue;
        }
        let own = sim.own;
        let driver = &mut sim.race.state_mut().karts[own].driver;
        let said = match *action {
            Action::DebugAddWeapon(item) => {
                vehicle::item::debug_roll(driver, item);
                format!("(debug) rolled item {item}")
            }
            Action::DebugSetWumpa(count) => {
                driver.num_wumpas = count as i8;
                format!("(debug) wumpa set to {count}")
            }
            // As crossing the line last does, first of all (`PlayLevel_UpdateLapStats`). R puts
            // the kart back on the grid, not finished.
            Action::DebugFx(crate::fx::Fx::Finish) => {
                driver.actions_flag_set |= vehicle::actions::RACE_FINISHED;
                driver.driver_rank = 0;
                "(debug) finished first".into()
            }
            _ => continue,
        };
        toasts.write(Toast::new(said, 2000));
    }
}

/// Runs `f` on the motion of each kart's drawing, with the kart of the sim that it follows.
/// (The kart we drive follows our slot, which is another one in each race.)
fn each_motion(
    sim: &crate::props::Sim,
    motions: &mut Query<(&mut KartMotion, &mut KartSlot, Has<Kart>)>,
    mut f: impl FnMut(&mut KartMotion, &race::Kart),
) {
    for (mut motion, mut slot, ours) in motions.iter_mut() {
        if ours {
            slot.0 = sim.own;
        }
        if let Some(kart) = sim.race.karts.get(slot.0) {
            f(&mut motion, kart);
        }
    }
}
