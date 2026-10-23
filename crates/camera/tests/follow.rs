//! Sanity checks of the follow camera that need no reference: it sits behind
//! a kart that drives straight, and swings out to the side of a drift.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use camera::CameraDc;
use coll::Collision;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use vehicle::sim::{spawn, tick};
use vehicle::{Driver, Env, Input, PhysState, buttons};

fn level(id: LevelId) -> Option<Level> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let file = b.big.read(&mut b.disc, idx.get()).unwrap();
    Some(Level::parse(&DramFile::parse(&file).unwrap()).unwrap())
}

/// The heading of the camera relative to straight behind the driver, in 1/4096 turns.
fn swing(camera: &CameraDc, d: &Driver) -> i32 {
    let (eye, at) = (camera.pb_pos, camera.look_at_pos);
    let seen = math::ratan2(eye.x as i32 - at.x, eye.z as i32 - at.z);
    ((seen - (d.angle as i32 + 0x800)) + 0x800).rem_euclid(0x1000) - 0x800
}

#[test]
fn swings_out_in_a_powerslide() {
    let Some(level) = level(LevelId::DINGO_CANYON) else {
        return;
    };
    let mut coll = Collision::default();
    let mut d = Driver::new(0, 0);
    d.anim_num_frames = [30, 12, 20, 16, 24, 8, 8, 8];
    spawn(&mut d, &mut coll, &level, 0);
    let mut camera = CameraDc::default();

    let mut straight = 0;
    let mut drifting = 0;
    let mut held = 0;
    for t in 0..300u32 {
        // Floor it; hop at 100 and keep the button held while steering left.
        let hop = (100..300).contains(&t);
        let new_held = buttons::CROSS | if hop { buttons::R1 } else { 0 };
        let input = Input {
            buttons_held: new_held,
            buttons_tapped: new_held & !held,
            stick_lx: if hop { 0 } else { 0x80 },
            ..Input::default()
        };
        held = new_held;
        let env = Env {
            elapsed_event_time: t as i32 * 32,
            timer: t,
            ..Env::default()
        };
        tick(&mut d, &mut coll, &level, &env, &input, &mut Vec::new());
        camera.follow_driver(
            &d,
            &level,
            &camera::Input {
                elapsed_time_ms: 32,
                ..Default::default()
            },
        );

        if t == 90 {
            straight = swing(&camera, &d);
        }
        if d.phys_state == PhysState::PowerSlide {
            drifting = drifting.max(swing(&camera, &d).abs());
        }
    }
    eprintln!("straight swing {straight}, largest swing in the drift {drifting}");
    assert!(
        straight.abs() < 40,
        "the camera should be behind a kart driving straight, was {straight}"
    );
    assert!(
        drifting > 100,
        "the camera should swing out in a powerslide, largest swing {drifting}"
    );
}
