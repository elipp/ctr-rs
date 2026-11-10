//! One game tick of the race: the pads of the others, rollback, and what follows from it.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use race::Pad;
use race::rollback::Step;
use vehicle::{Effect, buttons};

use super::follow::view_of;
use super::input::{Controller, pad_of};
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
    /// Our kart had finished last tick, and the played ticks left of the camera's hold at the
    /// finish line.
    finish: (bool, u32),
}

/// How long the camera stays at the finish line watching our kart coast away, in played ticks.
const FINISH_HOLD_TICKS: u32 = 60;
/// How far above our kart's position the held camera looks.
const FINISH_HOLD_LOOK_HEIGHT: i32 = 0x40;

/// One game tick of the sim.
pub(super) fn drive_tick(
    controller: Controller,
    track: Res<Track>,
    mut sim: ResMut<crate::props::Sim>,
    mut drive: ResMut<Drive>,
    mut hold: crate::fx::Hold,
    mut lobby: ResMut<crate::lobby::Lobby>,
    config: Res<crate::config::Config>,
    mut dummies: ResMut<crate::remote::DummyKarts>,
    shown: Shown,
    mut memory: Local<Memory>,
    #[cfg(feature = "brain")] mut brains: ResMut<crate::learned::Brains>,
) {
    let Shown {
        mut particles,
        mut motions,
        smoothing,
        mut net_label,
        mut toasts,
        mut sfx,
    } = shown;
    let Memory {
        was_paused,
        waits,
        finish,
    } = &mut *memory;
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
        drive.previous_view = drive.current_view;
        drive.weather.freeze();
        drive.confetti.freeze();
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
    // The lobby closed, or the race failed: back to the grid with our kart alone (and the dummy
    // karts of `/debug spawn-karts`, which also start it over when they change: in number, or
    // in who they are and whether they drive).
    if !online && (dummies.is_changed() || sim.race.karts.len() != 1 + dummies.0.len()) {
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
            // (`/debug spectate`: ours is driven as the dummies are, and we watch.)
            None => (
                std::iter::once(race::KartSpec {
                    bot: dummies.1.is_some() && !dummies.3,
                    line: dummies.2.filter(|_| dummies.1.is_some()),
                    ..drive.spec()
                })
                .chain(dummies.0.iter().copied())
                .collect(),
                dummies.1.unwrap_or(0),
                dummies.1.is_some(),
            ),
        };
        sim.restart(
            &track.0,
            &specs,
            own,
            lobby.online.as_ref().map(|net| net.laps),
            spectator,
        );
        // The dummies have left from tick 0: a neutral pad, and nothing to wait for. Unless the
        // policy drives them: then their pads are made here, each tick (below).
        if !online {
            let first = if dummies.1.is_some() { 0 } else { 1 };
            if dummies.3 {
                #[cfg(feature = "brain")]
                brains.start(&track.0, &sim.race, &specs, first..sim.race.players());
            } else {
                for slot in first..sim.race.players() {
                    sim.race.leave(slot);
                }
            }
        }
        particles.list.clear();
        drive.camera = camera::CameraDc::default();
        drive.weather = crate::weather::WeatherView::new(&track.0);
        drive.confetti = default();
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
    let spectating = match &lobby.online {
        Some(net) => net.spectating(),
        None => dummies.1.is_some(),
    };
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
    let late: Vec<usize> = if online {
        (0..sim.race.players())
            .filter(|&slot| spectating || slot != own)
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
    // Which way R1 and L1 were tapped this tick, to change the kart the camera follows.
    let mut cycle = 0;
    // A spectator has no pad to play, nor does a finished player that the sim ignores, but their
    // R1 and L1 pick the kart to follow all the same.
    if watches {
        let held = controller.read(waits.1).buttons_held;
        let taps = held & !waits.1;
        waits.1 = held;
        cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
    }
    // The pads of the karts the policy drives (`/debug spawn-karts <n> brain`), from the race as
    // it is now. Offline every pad is in before the tick plays: nothing is predicted.
    #[cfg(feature = "brain")]
    if !online && dummies.3 {
        for (slot, pad) in brains.pads(&track.0, &sim.race) {
            sim.race
                .confirm(slot, tick, pad)
                .expect("the policy's pads come in order");
        }
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
        } else if online {
            Pad::of_input(&controller.read(held))
        } else {
            Pad::of_input(&controller.read(held))
        };
        if !watches {
            let taps = pad.held & !held;
            cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
            // (So that an R1 held over the line is not taken for a tap once we watch.)
            waits.1 = pad.held;
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
        drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
    }

    // A boost pulls the camera back.
    if mine
        .effects
        .iter()
        .any(|e| matches!(e, Effect::CameraFireSpeedZoom { .. }))
    {
        drive.camera.flags |= camera::flags::FIRE_SPEED_ZOOM;
    }
    let camera_input = camera::Input {
        elapsed_time_ms: vehicle::Env::default().elapsed_time_ms,
        ..Default::default()
    };
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
            if !finished || net.watching >= players || net.gone[net.watching] {
                net.watching = own;
            }
            if finished && cycle != 0 {
                step(net, cycle);
            }
        }
        if net.watching != before {
            drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
        }
    } else if let Some(watched) = dummies.1
        && cycle != 0
    {
        // (`/debug spectate`: any kart, ours too. Not a change that starts the race over.)
        let players = sim.race.players() as i32;
        dummies.bypass_change_detection().1 =
            Some((watched as i32 + cycle).rem_euclid(players) as usize);
        drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
    }
    let watching = lobby
        .online
        .as_ref()
        .map_or(dummies.1.unwrap_or(own), |net| net.watching);
    // The HUD and the sounds follow the kart the spectator watches, as they follow ours.
    if spectating {
        sim.own = watching;
    }
    let kart = &sim.race.karts[watching];
    // Past the line the camera stays there a while and turns after our kart as it coasts away,
    // until R1 or L1 picks a kart to follow (or a rollback takes the finish back).
    let (was_finished, held) = finish;
    let ours_finished = !spectating && finished;
    if ours_finished && !*was_finished {
        *held = FINISH_HOLD_TICKS;
    }
    *was_finished = ours_finished;
    if !ours_finished || cycle != 0 || watching != own {
        *held = 0;
    }
    let holding = *held > 0;
    if holding {
        // (Counted in played ticks only: the wait for the others past the line holds still.)
        *held -= 1;
    }
    let jumped = !holding && drive.camera.flags & camera::flags::DIRECTION_CHANGED != 0;
    if holding {
        let mut target = kart.driver.pos_curr;
        target.x >>= 8;
        target.y = (target.y >> 8) + FINISH_HOLD_LOOK_HEIGHT;
        target.z >>= 8;
        drive.camera.look_at(target);
        // Once it is over, straight back behind the kart.
        drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
    } else {
        drive
            .camera
            .follow_driver(&kart.driver, &track.0, &camera_input);
    }
    // The start-line fly-in, over the chase camera, for as long as the race says.
    let fly_in_frame = camera::TRANSITION_FRAMES - tick as i32;
    if sim.race.intro_ticks > 0 && fly_in_frame >= 0 {
        drive.camera.fly_in(&track.0, fly_in_frame);
    }
    drive.weather.step(&track.0, &drive.camera, jumped);
    let ours = &sim.race.karts[sim.own].driver;
    let won = ours.actions_flag_set & vehicle::actions::RACE_FINISHED != 0 && ours.driver_rank == 0;
    drive
        .confetti
        .step(&drive.camera, hold.fx.confetti.apply(won));

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
    drive.previous_view = drive.current_view;
    drive.current_view = view_of(&drive.camera);

    // The tick's sounds, the other karts' heard from where the camera is now.
    let places: Vec<Vec3> = sim
        .race
        .karts
        .iter()
        .map(|kart| pose_of(&kart.driver).translation)
        .collect();
    let ear = drive.current_view.transform();
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
