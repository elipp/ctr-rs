//! One game tick of the race: demos, the pads of the others, rollback, and what follows from it.

use bevy::prelude::*;
use race::rollback::Step;
use race::Pad;
use vehicle::demo::Demo;
use vehicle::{Effect, buttons};

use super::follow::view_of;
use super::input::{pad_of, read_input};
use super::overlay::DemoHud;
use super::pose::{Pose, pose_of};
use super::{DEMO_FILE, DemoMode, Drive, Kart};
use crate::Track;
use crate::command::Action;
use crate::kart::{KartMotion, KartSlot};
use crate::toast::Toast;

/// How many ticks we may play past the last pad of another player before we wait for it: the
/// prediction is that long at most. The clock is kept with the others' without this (see
/// `lobby::pace`), so it only matters when one of them stalls for good: 2 seconds.
const MAX_LEAD: u32 = 60;

/// How often the toast "Waiting for ..." is put up again while we wait: every 3 seconds, and it
/// stays that long.
const WAIT_TOAST_TICKS: u32 = 90;

/// How long F7 and F8 freeze an online client for.
const STALLS_MS: [u64; 2] = [500, 2000];


/// One game tick of the sim.
pub(super) fn drive_tick(
    mut particles: ResMut<crate::exhaust::Particles>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    track: Res<Track>,
    mut sim: ResMut<crate::props::Sim>,
    mut drive: ResMut<Drive>,
    mut hold: crate::fx::Hold,
    mut was_paused: Local<bool>,
    mut keys_held: Local<[bool; 4]>,
    mut demo_label: Query<&mut Text, With<DemoHud>>,
    mut lobby: ResMut<crate::lobby::Lobby>,
    mut motions: Query<(&mut KartMotion, &mut KartSlot, Has<Kart>)>,
    smoothing: Res<crate::kart::Smoothing>,
    mut toasts: MessageWriter<Toast>,
    // The ticks waited for a late player, and the buttons a spectator held last tick.
    mut waits: Local<(u32, u32)>,
    mut sfx: crate::sound::Sfx,
) {
    let drive = &mut *drive;
    // In an online race the sim is everyone's: no pause, no restarts, no demos.
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

    // F5 records a demo from the grid (and saves it when pressed again), F6 plays the saved
    // one. R (or the menu's RESTART) is the grid without either. A demo is the inputs alone, so
    // every start puts everything, the objects too, back as it was at tick 0.
    let picked = std::mem::take(&mut hold.menu.restart);
    let mut restart = (keys.pressed(KeyCode::KeyR) || picked) && !online;
    // Edges are found here, not with `just_pressed`: that is cleared every rendered frame, and
    // a frame can pass without a tick.
    let demo_down = [
        !online && keys.any_pressed([KeyCode::F5]) || !online && keys.pressed(KeyCode::Digit1),
        !online && keys.any_pressed([KeyCode::F6]) || !online && keys.pressed(KeyCode::Digit2),
    ];
    let [record_pressed, play_pressed] = std::array::from_fn(|i| demo_down[i] && !keys_held[i]);
    keys_held[..2].copy_from_slice(&demo_down);
    if record_pressed {
        if let DemoMode::Recording(demo) = &drive.demo {
            match std::fs::write(DEMO_FILE, demo.to_text()) {
                Ok(()) => info!("saved {} ticks to {DEMO_FILE}", demo.length),
                Err(e) => error!("could not save {DEMO_FILE}: {e}"),
            }
            drive.demo = DemoMode::Off;
        } else {
            drive.demo = DemoMode::Recording(Demo::new(drive.character));
            restart = true;
        }
    } else if play_pressed {
        match std::fs::read_to_string(DEMO_FILE)
            .map_err(|e| e.to_string())
            .and_then(|text| Demo::from_text(&text))
        {
            Ok(demo) => {
                // The demo was driven by this character.
                drive.character = demo.character;
                drive.demo = DemoMode::Playing(demo);
                restart = true;
            }
            Err(e) => error!("could not play {DEMO_FILE}: {e}"),
        }
    } else if restart {
        drive.demo = DemoMode::Off;
    }
    // The lobby closed, or the race failed: back to the grid with our kart alone.
    if !online && sim.race.karts.len() != 1 {
        restart = true;
    }
    // The race starts: tick 0 for everyone.
    if let Some(net) = lobby.online.as_mut().filter(|net| net.fresh) {
        net.fresh = false;
        drive.demo = DemoMode::Off;
        restart = true;
    }
    if restart {
        let (specs, own, spectator) = match &lobby.online {
            Some(net) => (net.specs.clone(), net.viewed(), net.spectating()),
            None => (vec![drive.spec()], 0, false),
        };
        sim.restart(&track.0, &specs, own, lobby.online.is_some(), spectator);
        particles.list.clear();
        drive.camera = camera::CameraDc::default();
        drive.weather = crate::weather::WeatherView::new(&track.0);
        drive.confetti = default();
        each_motion(&sim, &mut motions, |motion, kart| motion.reset(kart));
    }

    // F7 and F8 freeze this client for a moment, to see the race put right after a hitch: the
    // clock does not catch up (the frame is cut short), so the others run ahead and we wait.
    let stall_down = [KeyCode::F7, KeyCode::F8].map(|key| online && keys.pressed(key));
    for (i, millis) in STALLS_MS.into_iter().enumerate() {
        if stall_down[i] && !keys_held[2 + i] {
            debug!("(debug) stalling for {millis} ms");
            std::thread::sleep(std::time::Duration::from_millis(millis));
        }
    }
    keys_held[2..].copy_from_slice(&stall_down);

    // Tick 0 has to come first: the others must be ready as well.
    if lobby.online.as_ref().is_some_and(|net| !net.go) {
        for mut text in &mut demo_label {
            text.0 = lobby
                .online
                .as_ref()
                .map_or_else(String::new, |net| net.summary(0, 0));
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
    // Each client runs its own 30 Hz clock, so one that is ahead would stay ahead (up to the
    // rollback window) and predict the others further and further. It waits for them instead.
    let late: Vec<usize> = if online {
        (0..sim.race.players())
            .filter(|&slot| spectating || slot != own)
            .filter(|&slot| tick > sim.race.confirmed_until(slot).saturating_add(MAX_LEAD))
            .collect()
    } else {
        Vec::new()
    };
    if !late.is_empty() {
        // (The host removes a player that stays silent: see `net::KICK_AFTER`.)
        if waits.0 % WAIT_TOAST_TICKS == 0
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
    // A spectator has no pad to play, but its R1 and L1 pick the kart to follow all the same.
    if spectating {
        let held = read_input(&keys, &pads, waits.1).buttons_held;
        let taps = held & !waits.1;
        waits.1 = held;
        cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
    }
    // Our own pad for this tick, from the controller as it is now. It is played at once, and the
    // others get it as well. (A tick that waits plays the pad it already has.)
    if !spectating && sim.race.confirmed_until(own) <= tick {
        let held = sim.kart().held;
        let pad = if online && hold.menu.open {
            // The menu has the controls: the kart coasts.
            Pad::default()
        } else if online {
            Pad::of_input(&read_input(&keys, &pads, held))
        } else {
            let mut input = match &drive.demo {
                DemoMode::Playing(demo) => demo.input(tick, held),
                _ => None,
            };
            if input.is_none() {
                if matches!(drive.demo, DemoMode::Playing(_)) {
                    info!("demo over after {tick} ticks");
                    drive.demo = DemoMode::Off;
                }
                input = Some(read_input(&keys, &pads, held));
            }
            let input = input.unwrap();
            if let DemoMode::Recording(demo) = &mut drive.demo {
                demo.record(tick, &input);
            }
            Pad::of_input(&input)
        };
        let taps = pad.held & !held;
        cycle = (taps & buttons::R1 != 0) as i32 - (taps & buttons::L1 != 0) as i32;
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
    for mut text in &mut demo_label {
        text.0 = match &drive.demo {
            _ if online => lobby
                .online
                .as_ref()
                .map_or_else(String::new, |net| net.summary(tick, sim.race.unsettled())),
            DemoMode::Off => String::new(),
            DemoMode::Recording(_) => format!("REC {tick}"),
            DemoMode::Playing(demo) => format!("DEMO {tick}/{}", demo.length),
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
    let heights: Vec<i32> = sim.race.karts.iter().map(|k| k.driver.inst_pos[1]).collect();
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
        if spectating {
            // Any kart of the race that is still here, whenever.
            if cycle != 0 || net.gone[net.watching] {
                let step = if cycle == 0 { 1 } else { cycle };
                for _ in 0..players {
                    net.watching = (net.watching as i32 + step).rem_euclid(players as i32) as usize;
                    if !net.gone[net.watching] {
                        break;
                    }
                }
            }
        } else if !finished || net.watching >= players || net.gone[net.watching] {
            net.watching = own;
        }
        if !spectating && finished && cycle != 0 {
            for _ in 0..players {
                net.watching = (net.watching as i32 + cycle).rem_euclid(players as i32) as usize;
                if !net.gone[net.watching] {
                    break;
                }
            }
        }
        if net.watching != before {
            drive.camera.flags |= camera::flags::DIRECTION_CHANGED;
        }
    }
    let watching = lobby.online.as_ref().map_or(own, |net| net.watching);
    // The HUD and the sounds follow the kart the spectator watches, as they follow ours.
    if spectating {
        sim.own = watching;
    }
    let kart = &sim.race.karts[watching];
    let jumped = drive.camera.flags & camera::flags::DIRECTION_CHANGED != 0;
    drive
        .camera
        .follow_driver(&kart.driver, &track.0, &camera_input);
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
/// sets the wumpa count. Not online, and not in a demo, which is the inputs alone.
pub(super) fn debug_actions(
    mut actions: MessageReader<Action>,
    mut sim: ResMut<crate::props::Sim>,
    drive: Res<Drive>,
    lobby: Res<crate::lobby::Lobby>,
    mut toasts: MessageWriter<Toast>,
) {
    for action in actions.read() {
        // Like F7 and F8, for as long as asked: the frame is cut short, so the others run ahead.
        if let Action::DebugStall(time) = action {
            if lobby.online.is_some() {
                debug!("(debug) stalling for {time:?}");
                std::thread::sleep(*time);
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
        if lobby.online.is_some() || !matches!(drive.demo, DemoMode::Off) {
            toasts.write(
                Toast::new("Not online or in a demo", 2500).color(Color::srgb(1.0, 0.4, 0.4)),
            );
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

