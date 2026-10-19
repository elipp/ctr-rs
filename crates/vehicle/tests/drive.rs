//! Sanity checks of the driving physics that need no reference: a kart on a
//! real track accelerates, stays on the ground, and steers.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use coll::Collision;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use vehicle::sim::{spawn, tick};
use vehicle::{Driver, Env, Input, actions, buttons};

fn level(id: LevelId) -> Option<Level> {
    let mut b = common::open_bigfile()?;
    let idx = BigIndex::level(id, Lod::OnePlayer, LevFile::Lev);
    let file = b.big.read(&mut b.disc, idx.get()).unwrap();
    Some(Level::parse(&DramFile::parse(&file).unwrap()).unwrap())
}

struct Drive {
    driver: Driver,
    coll: Collision,
    tick: u32,
}

impl Drive {
    fn new(level: &Level, character: usize) -> Drive {
        let mut coll = Collision::default();
        let mut driver = Driver::new(0, character);
        spawn(&mut driver, &mut coll, level, 0);
        Drive { driver, coll, tick: 0 }
    }

    fn step(&mut self, level: &Level, held: u32, lx: u8) {
        let env = Env { elapsed_event_time: self.tick as i32 * 32, timer: self.tick, ..Env::default() };
        let input = Input { buttons_held: held, buttons_tapped: 0, stick_lx: lx, ..Input::default() };
        tick(&mut self.driver, &mut self.coll, level, &env, &input, &mut Vec::new());
        self.tick += 1;
    }
}

#[test]
fn accelerates_along_the_track() {
    let Some(level) = level(LevelId::DINGO_CANYON) else { return };
    let mut drive = Drive::new(&level, 0);
    let start = drive.driver.pos_curr;
    let mut grounded = 0;
    let mut top_speed = 0;
    for t in 0..240 {
        drive.step(&level, buttons::CROSS, 0x80);
        grounded += (drive.driver.actions_flag_set & actions::TOUCH_GROUND != 0) as u32;
        top_speed = top_speed.max(drive.driver.speed_approx);
        if t % 20 == 0 {
            let d = &drive.driver;
            eprintln!("tick {t}: pos {:?} speed {} base {} state {:?}", d.pos_curr, d.speed_approx, d.base_speed, d.phys_state);
        }
    }
    let d = &drive.driver;
    eprintln!("start {start:?}\nend {:?} speed {} approx {} angle {} grounded {grounded}/240", d.pos_curr, d.speed, d.speed_approx, d.angle);
    assert!(grounded > 200, "the kart should be on the track, was grounded {grounded} of 240 ticks");
    assert!(top_speed > 10000, "the kart should speed up towards 13140, top speed {top_speed}");
}

#[test]
fn steering_turns_the_kart() {
    let Some(level) = level(LevelId::DINGO_CANYON) else { return };
    let angle_after = |lx: u8| {
        let mut drive = Drive::new(&level, 0);
        for _ in 0..30 {
            drive.step(&level, buttons::CROSS, 0x80);
        }
        let before = drive.driver.angle as i32;
        for _ in 0..20 {
            drive.step(&level, buttons::CROSS, lx);
        }
        (drive.driver.angle as i32 - before + 0x800).rem_euclid(0x1000) - 0x800
    };
    let (left, right) = (angle_after(0), angle_after(0xff));
    eprintln!("turn left {left}, turn right {right}");
    assert!(left != 0 && right != 0 && (left > 0) != (right > 0), "left {left}, right {right}");
}
