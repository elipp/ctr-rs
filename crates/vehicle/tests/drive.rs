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

/// Drops the kart from `height` (world units) above the start line and returns
/// the longest hang time and the reserves it gets on landing.
fn drop_from(level: &Level, height: i32) -> (i32, i32, u8) {
    let mut drive = Drive::new(level, 0);
    drive.driver.pos_curr.y += height << 8;
    drive.driver.pos_prev = drive.driver.pos_curr;
    // Only a jump (a hop or a ramp) counts as hang time: the airborne flag needs the jump cooldown.
    drive.driver.jump_cooldown_ms = 0x180;
    let mut hang = 0;
    let mut reserves = 0;
    let mut kart_state = 0;
    for _ in 0..200 {
        drive.step(level, 0, 0x80);
        let d = &drive.driver;
        hang = hang.max(d.jump_meter as i32);
        reserves = reserves.max(d.reserves as i32);
        kart_state = d.kart_state;
    }
    (hang, reserves, kart_state)
}

#[test]
fn long_hang_time_gives_a_boost() {
    let Some(level) = level(LevelId::DINGO_CANYON) else { return };
    let (short_hang, short_reserves, _) = drop_from(&level, 0x20);
    let (long_hang, long_reserves, _) = drop_from(&level, 0x900);
    eprintln!("short drop: hang {short_hang} ms, reserves {short_reserves}; long drop: hang {long_hang} ms, reserves {long_reserves}");
    assert!(short_hang < 640 && short_reserves == 0, "a short drop gives no boost");
    assert!(long_hang >= 640 && long_reserves > 0, "a long drop gives a boost: hang {long_hang}, reserves {long_reserves}");
}

/// Falls off the track at speed: the mask grabs the kart, puts it back on the track at the
/// respawn point of the last quad block it was on, lowers it, and the player drives again.
#[test]
fn a_fall_off_the_track_ends_with_a_respawn_on_the_track() {
    use vehicle::{PhysState, kart_state};

    let Some(level) = level(LevelId::DINGO_CANYON) else { return };
    let mut drive = Drive::new(&level, 0);
    for _ in 0..120 {
        drive.step(&level, buttons::CROSS, 0x80);
    }
    let last_valid = drive.driver.last_valid.expect("the kart has been on the track");
    let node = level.restart_points[level.quad_blocks[last_valid as usize].checkpoint_index as usize];

    // Dropped into the void.
    drive.driver.pos_curr.y -= 0x400 << 8;
    drive.driver.pos_prev = drive.driver.pos_curr;
    drive.driver.velocity.y = 0;
    let mut grabbed_ticks = 0;
    let mut revved_ticks = 0;
    let mut respawned_at = None;
    for _ in 0..200 {
        let before = drive.driver.phys_state;
        drive.step(&level, buttons::CROSS, 0x80);
        let d = &drive.driver;
        grabbed_ticks += (d.phys_state == PhysState::MaskGrab) as u32;
        revved_ticks += (d.phys_state == PhysState::RevEngine) as u32;
        if before == PhysState::MaskGrab && d.phys_state == PhysState::RevEngine {
            respawned_at = Some(d.pos_curr);
            assert_eq!(d.speed, 0);
            assert_eq!(d.kart_state, kart_state::ENGINE_REVVING);
            assert!(d.pos_curr.y > d.quad_block_height, "the mask holds the kart above the track");
        }
        assert!(d.kart_state != kart_state::MASK_GRABBED || d.phys_state == PhysState::MaskGrab);
    }
    let d = &drive.driver;
    eprintln!("grabbed {grabbed_ticks} ticks, revved {revved_ticks}, respawned at {respawned_at:?}, node {:?}, now {:?} {:?}", node.pos, d.pos_curr, d.phys_state);

    // The mask grab holds for 1440 ms (45 ticks), the rev lowers the kart from 0x8000 to 0x4000 in 32.
    assert!((44..=46).contains(&grabbed_ticks), "the grab lasts 1440 ms, was {grabbed_ticks} ticks");
    assert!((30..=34).contains(&revved_ticks), "the mask lowers the kart in about 32 ticks, was {revved_ticks}");
    let at = respawned_at.expect("the kart was put back on the track");
    assert_eq!((at.x >> 8, at.z >> 8), (node.pos.x as i32, node.pos.z as i32), "at the checkpoint of the last quad block");
    assert_eq!(d.phys_state, PhysState::Driving);
    assert!(d.mask.is_none(), "the mask has gone");
    assert!(d.actions_flag_set & actions::TOUCH_GROUND != 0, "on the track again");
}
