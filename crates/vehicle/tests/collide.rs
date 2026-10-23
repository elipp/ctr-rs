//! Differential test of kart-against-kart collisions: random karts close to each other, through
//! this crate's `force::collide_drivers` and through the reference C code
//! (`tests/reference/collide.c`: the real `VehPhysForce_CollideDrivers`, `PROC_CollidePointWithBucket`,
//! `VehPhysCrash_AnyTwoCars`, `VehPhysCrash_BounceSelf` and `VehPhysCrash_Attack`), and requires
//! the same velocities, flags, pending damage, effects and bump sound timing afterwards.
//!
//! Skipped (with a note) if the reference is unavailable. `VEHICLE_COLLIDE_TRIALS` overrides the
//! number of trials.

#[path = "../../coll/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use std::fmt::Write as _;
use std::path::Path;

use support::{Rng, build_driver, run_reference};
use vehicle::collide::{Feedback, Others};
use vehicle::force::collide_drivers;
use vehicle::{Driver, Effect, Env, actions, baked, forced_jump, kart_state};

fn effect_line(e: &Effect) -> String {
    match *e {
        Effect::DriverCrashing { echo, volume } => format!(" C {} {volume}", echo as u8),
        Effect::Voiceline {
            id,
            driver_id,
            priority,
        } => format!(" V {id} {driver_id} {priority}"),
        Effect::JogCon {
            driver_id,
            pattern,
            strength,
        } => format!(" J {driver_id} {pattern} {strength}"),
        Effect::ShockFreq { driver_id, a, b } => format!(" F {driver_id} {a} {b}"),
        Effect::ShockForce { driver_id, a, b } => format!(" G {driver_id} {a} {b}"),
        other => panic!("unexpected effect {other:?}"),
    }
}

fn random_driver(rng: &mut Rng, id: u8, near: Option<&Driver>) -> Driver {
    let mut d = Driver::new(id, id.into());
    d.driver_id = id;
    d.stats.collision_weight = [256, 200, 300, 150, 400][rng.below(5) as usize];
    let big = rng.below(3) == 0;
    let speed = if big { 0x6000 } else { 0x1800 };
    d.velocity.x = rng.range(-speed, speed);
    d.velocity.y = rng.range(-speed / 4, speed / 4);
    d.velocity.z = rng.range(-speed, speed);
    d.accel.x = rng.range(-0x400, 0x400) as i16;
    d.accel.y = rng.range(-0x400, 0x400) as i16;
    d.accel.z = rng.range(-0x400, 0x400) as i16;
    d.pos_curr.x = rng.range(-0x400000, 0x400000);
    d.pos_curr.y = rng.range(-0x100000, 0x100000);
    d.pos_curr.z = rng.range(-0x400000, 0x400000);
    if let Some(first) = near {
        // Within (and a little beyond) the reach of 0x80 units: 0x80 << 8 in 24.8 fixed point.
        let reach = if rng.below(4) == 0 {
            0x90 << 8
        } else {
            0x50 << 8
        };
        d.pos_curr.x = first.pos_curr.x + rng.range(-reach, reach);
        d.pos_curr.y = first.pos_curr.y + rng.range(-reach, reach);
        d.pos_curr.z = first.pos_curr.z + rng.range(-reach, reach);
    }
    let flags = [
        actions::MASK_WEAPON,
        actions::TURBO_ITEM,
        actions::ENGINE_ECHO,
    ];
    for f in flags {
        if rng.below(3) == 0 {
            d.actions_flag_set |= f;
        }
    }
    d.reserves = if rng.below(2) == 0 {
        0
    } else {
        rng.range(1, 0x3c0) as i16
    };
    d.kart_state = [
        kart_state::NORMAL,
        kart_state::NORMAL,
        kart_state::BLASTED,
        kart_state::DRIFTING,
    ][rng.below(4) as usize];
    d.simp_turn_state = rng.range(-1, 1) as i8;
    d.invincible_timer = if rng.below(6) == 0 {
        rng.range(1, 100)
    } else {
        0
    };
    d.forced_jump_type = forced_jump::NONE;
    d.collision_disabled = rng.below(12) == 0;
    d.screen_offset_y = rng.range(-8, 8) as i8;
    d
}

fn driver_line(i: usize, d: &Driver) -> String {
    let at = baked::inst_matrix(d).1;
    let values = [
        i as i64,
        d.driver_id as i64,
        d.kart_state as i64,
        d.simp_turn_state as i64,
        d.pos_curr.x as i64,
        d.pos_curr.y as i64,
        d.pos_curr.z as i64,
        d.velocity.x as i64,
        d.velocity.y as i64,
        d.velocity.z as i64,
        d.accel.x as i64,
        d.accel.y as i64,
        d.accel.z as i64,
        d.stats.collision_weight as i64,
        d.actions_flag_set as i64,
        d.reserves as i64,
        d.invincible_timer as i64,
        d.forced_jump_type as i64,
        d.collision_disabled as i64,
        at[0] as i64,
        at[1] as i64,
        at[2] as i64,
    ];
    format!(
        "D{}\n",
        values.iter().map(|v| format!(" {v}")).collect::<String>()
    )
}

fn summary(d: &Driver) -> String {
    format!(
        "{} {} {} {} {} {} {} {} 0 |",
        d.velocity.x,
        d.velocity.y,
        d.velocity.z,
        d.actions_flag_set,
        d.forced_jump_type,
        d.pending_damage_type,
        d.pending_damage_reason,
        d.pending_damage_attacker.map_or(-1, i32::from),
    )
}

#[test]
fn matches_reference() {
    let driver_c = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/collide.c");
    let Some(exe) = build_driver(
        &driver_c,
        "vehicle_collide",
        &[
            "game/PROC.c",
            "game/MATH/MATH_2_FastSqrt.c",
            "game/Vehicle/VehCalc.c",
            "game/Vehicle/VehPhysForce.c",
            "game/Vehicle/VehPhysCrash.c",
        ],
    ) else {
        return;
    };

    let trials: usize = std::env::var("VEHICLE_COLLIDE_TRIALS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let mut rng = Rng(0x51ab_c3d7_9e02_4f18);

    let mut script = String::new();
    let mut expected = Vec::new();
    // (bumps, bumps that were heard, hard crashes, mask hits, boost hits, pairs out of reach, disabled)
    let mut seen = [0u32; 7];

    for _ in 0..trials {
        let n = rng.range(2, 4) as usize;
        let first = random_driver(&mut rng, 0, None);
        let mut drivers = vec![first.clone()];
        for i in 1..n {
            let near = if rng.below(8) == 0 {
                None
            } else {
                Some(&first)
            };
            drivers.push(random_driver(&mut rng, i as u8, near));
        }
        // Two karts the same distance away: the first in the list is the one that is bumped.
        if n >= 3 && rng.below(5) == 0 {
            drivers[2].pos_curr = drivers[1].pos_curr;
            drivers[2].screen_offset_y = drivers[1].screen_offset_y;
            drivers[2].collision_disabled = false;
            drivers[1].collision_disabled = false;
        }
        let timer = rng.range(10, 1000);
        let last = timer - rng.range(0, 6);

        writeln!(script, "G {timer} {last}").unwrap();
        for (i, d) in drivers.iter().enumerate() {
            script.push_str(&driver_line(i, d));
        }
        writeln!(script, "X {n}").unwrap();

        // The Rust side.
        let env = Env {
            timer: timer as u32,
            ..Env::default()
        };
        let mut feedback = Feedback { last_frame: last };
        let mut effects = Vec::new();
        let (head, tail) = drivers.split_at_mut(1);
        let mut later: Vec<&mut Driver> = tail.iter_mut().collect();
        collide_drivers(
            &mut head[0],
            &mut Others {
                drivers: &mut later,
                feedback: &mut feedback,
            },
            &env,
            &mut effects,
        );

        let mut line: String = drivers.iter().map(|d| summary(d) + " ").collect();
        line.push_str(&format!(
            "{} | {}",
            effects.iter().map(effect_line).collect::<String>(),
            feedback.last_frame
        ));
        expected.push(line);

        if drivers
            .iter()
            .any(|d| d.actions_flag_set & actions::HUMAN_HUMAN_COLLISION != 0)
        {
            seen[0] += 1;
        }
        if effects
            .iter()
            .any(|e| matches!(e, Effect::DriverCrashing { .. }))
        {
            seen[1] += 1;
        }
        if effects
            .iter()
            .any(|e| matches!(e, Effect::Voiceline { id: 5, .. }))
        {
            seen[2] += 1;
        }
        if drivers.iter().any(|d| d.pending_damage_type == 2) {
            seen[3] += 1;
        }
        if drivers.iter().any(|d| d.pending_damage_type == 3) {
            seen[4] += 1;
        }
    }

    let reference = run_reference(&exe, &script);
    assert_eq!(reference.len(), expected.len());
    let mut failures = 0;
    // The two sides space the fields apart a little differently.
    fn words(line: &str) -> Vec<Vec<&str>> {
        line.split('|')
            .map(|part| part.split_whitespace().collect())
            .collect()
    }
    for (t, (got, want)) in expected.iter().zip(&reference).enumerate() {
        if words(got) != words(want) {
            failures += 1;
            if failures <= 5 {
                eprintln!("trial {t}:\n  rust {got}\n  c    {want}");
            }
        }
    }
    eprintln!(
        "coverage (heard bumps, hard crashes, mask hits, boost hits, bumps): {:?}",
        &seen[..5]
    );
    assert_eq!(failures, 0, "{failures} of {trials} trials differ");
    assert!(
        seen[0] > trials as u32 / 20 && seen[1] > 0 && seen[3] > 0 && seen[4] > 0,
        "the trials do not cover the bumps: {seen:?}"
    );
}
