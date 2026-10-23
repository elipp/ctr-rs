//! `RB_CrateAny_*`: the crates of the track (fruit, weapon and time crates). A hit breaks one: it
//! shrinks to nothing at once, waits about a second, and grows back to full size (a time crate
//! does not come back). Its thread exists only from the first touch until it is whole again.
//!
//! What breaking one gives a driver is done for the fruit crate (5 to 8 wumpa for the pickup HUD
//! to give) and the weapon crate (the item roulette, see `vehicle::item`). The time crate's bonus
//! is not here.

use coll::model_id;
use vehicle::{Driver, item, wumpa};

use crate::{Context, Instance, Tick, Touch};

/// `crateInst->scale`, 0x1000 being full size.
pub const FULL_SCALE: i16 = 0x1000;
/// `crateObj->cooldown` after a hit: ticks to wait, which the game says is about a second.
const COOLDOWN_TICKS: i32 = 0x1e;
/// How much a crate grows a tick, so 16 ticks to full size.
const GROW_STEP: i16 = 0x100;

/// Which `RB_Crate*_LInC` a crate has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `PU_FRUIT_CRATE`
    Fruit,
    /// `PU_RANDOM_CRATE`: the weapon box.
    Weapon,
    /// `STATIC_TIME_CRATE_01..03`
    Time,
}

impl Kind {
    /// The kind of a level model (`InstDef::model_id`).
    pub fn of_model(model_id: i32) -> Option<Kind> {
        match model_id {
            7 => Some(Kind::Fruit),
            8 => Some(Kind::Weapon),
            0x5c | 0x64 | 0x65 => Some(Kind::Time),
            _ => None,
        }
    }
}

impl Kind {
    /// The `colorRGBA` of the break-up of a crate of this kind: white for a weapon box, orange for
    /// a fruit crate, green for a time crate (`render::explosion::tint` reads it).
    pub fn explosion_color(self) -> u32 {
        match self {
            Kind::Weapon => 0x0faf_afa0,
            Kind::Fruit => 0x0f29_53a0,
            Kind::Time => 0x080f_f000,
        }
    }

    /// Whether the break-up is turned by a random angle around the vertical: all but the fruit
    /// crate's.
    pub fn explosion_turned(self) -> bool {
        self != Kind::Fruit
    }
}

/// A crate: `struct Crate` and its instance's scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateBox {
    pub kind: Kind,
    /// `scale.x` (the three are the same): 0 broken, [`FULL_SCALE`] whole, between growing back.
    pub scale: i16,
    /// `thread != NULL`: born at the first touch.
    thread: bool,
    cooldown: i32,
    /// `boolPauseCooldown`: a time crate keeps it, so that it stays broken.
    pause_cooldown: bool,
    /// How many times it was broken, for the explosion (the caller counts the change).
    pub breaks: u32,
}

impl CrateBox {
    pub fn new(kind: Kind) -> CrateBox {
        CrateBox {
            kind,
            scale: FULL_SCALE,
            thread: false,
            cooldown: 0,
            pause_cooldown: false,
            breaks: 0,
        }
    }

    /// `RB_Crate*_LInC` (with `RB_Crate*_ThCollide`): a driver touched it, with `model`
    /// (`Input1.modelID`). Breaks it if it is whole; 1 if so, otherwise 0. A touch while it is
    /// gone starts the wait again. `random_number` is `sdata->randomNumber`.
    pub fn linc(&mut self, driver: &mut Driver, model: i16, random_number: &mut i32) -> i32 {
        if !self.thread {
            // `RB_CrateAny_LInC_Birth` and `_Init`.
            self.thread = true;
            self.cooldown = 0;
            self.pause_cooldown = false;
        }
        if self.cooldown == 0 && (self.scale == 0 || self.scale == FULL_SCALE) {
            self.cooldown = COOLDOWN_TICKS;
            if self.scale == FULL_SCALE {
                self.scale = 0;
                self.breaks += 1;
                // A time crate is spent for good.
                self.pause_cooldown |= self.kind == Kind::Time;
                match self.kind {
                    Kind::Fruit => self.give_wumpa(driver, model, random_number),
                    // A bomb, missile or shield that breaks the crate gives its owner the roulette
                    // in retail; the weapons are not ported, so only a driver's touch counts.
                    Kind::Weapon if model == model_id::PLAYER => item::start_roulette(driver),
                    _ => {}
                }
                return 1;
            }
        }
        0
    }

    /// `RB_CrateFruit_ThCollide`'s reward: breaking the crate advances the RNG whoever hits it, and
    /// 5 to 8 wumpa go into the player's pickup HUD queue (replacing what was in it).
    fn give_wumpa(&self, driver: &mut Driver, model: i16, random_number: &mut i32) {
        let random = math::mix_rng_scramble(random_number);
        if model == model_id::PLAYER {
            driver.pickup_wumpa_cooldown = wumpa::FLIGHT_TICKS;
            driver.pickup_wumpa_collected = random % 4 + 5;
        }
    }

    /// `RB_CrateAny_ThTick_Grow`, one game tick, while the crate has its thread.
    pub fn tick(&mut self) {
        if !self.thread {
            return;
        }
        if self.kind == Kind::Time {
            // It does not grow back: its thread ends at once, with the wait run for this tick.
            self.thread = false;
        }
        if self.cooldown != 0 {
            if !self.pause_cooldown {
                self.cooldown -= 1;
            }
        } else if self.scale < FULL_SCALE {
            self.scale += GROW_STEP;
        } else {
            self.scale = FULL_SCALE;
            self.thread = false;
        }
    }
}

impl Instance for CrateBox {
    fn th_tick(&mut self, _cx: &Context, _out: &mut Tick) {
        self.tick();
    }

    fn touch(&mut self, hit: &mut Touch) -> i32 {
        self.linc(hit.driver, hit.model, hit.random_number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: i16 = model_id::PLAYER;

    fn tick_n(c: &mut CrateBox, n: usize) {
        for _ in 0..n {
            c.tick();
        }
    }

    #[test]
    fn a_hit_breaks_it_and_it_grows_back_after_half_a_second_and_a_bit() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut c = CrateBox::new(Kind::Weapon);
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 1);
        assert_eq!((c.scale, c.breaks), (0, 1));

        // Thirty ticks of waiting, then sixteen of growing.
        tick_n(&mut c, 30);
        assert_eq!(c.scale, 0);
        tick_n(&mut c, 1);
        assert_eq!(c.scale, 0x100);
        tick_n(&mut c, 14);
        assert_eq!(c.scale, 0xf00);
        tick_n(&mut c, 1);
        assert_eq!(c.scale, FULL_SCALE);
        // The tick after, its thread ends, and it can be broken again.
        tick_n(&mut c, 1);
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 1);
        assert_eq!(c.breaks, 2);
    }

    #[test]
    fn it_cannot_be_hit_while_waiting_or_growing() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut c = CrateBox::new(Kind::Fruit);
        c.linc(&mut driver, PLAYER, &mut rng);
        // Waiting.
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 0);
        tick_n(&mut c, 30 + 5);
        assert_eq!(c.scale, 5 * 0x100);
        // Growing: no hit, and it does not stop growing.
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 0);
        tick_n(&mut c, 11);
        assert_eq!(c.scale, FULL_SCALE);
    }

    #[test]
    fn touching_the_gap_between_the_wait_and_the_growth_restarts_the_wait() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut c = CrateBox::new(Kind::Weapon);
        c.linc(&mut driver, PLAYER, &mut rng);
        tick_n(&mut c, 30);
        // Waiting is over and it has not started growing: whatever sits there restarts it.
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 0);
        tick_n(&mut c, 30);
        assert_eq!(c.scale, 0);
        tick_n(&mut c, 1);
        assert_eq!(c.scale, 0x100);
    }

    #[test]
    fn a_fruit_crate_gives_five_to_eight_wumpa_and_replaces_the_queue() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            let mut c = CrateBox::new(Kind::Fruit);
            driver.pickup_wumpa_collected = 20;
            assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 1);
            assert!((5..=8).contains(&driver.pickup_wumpa_collected));
            assert_eq!(driver.pickup_wumpa_cooldown, wumpa::FLIGHT_TICKS);
            seen.insert(driver.pickup_wumpa_collected);
        }
        assert_eq!(seen, [5, 6, 7, 8].into());

        // The first break of a game: 100 -> (100 * 0x6255 + 0x3619) & 0xffff = 2531149 mod 65536 = 40781.
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut c = CrateBox::new(Kind::Fruit);
        c.linc(&mut driver, PLAYER, &mut rng);
        assert_eq!(rng, 40781);
        assert_eq!(driver.pickup_wumpa_collected, 40781 % 4 + 5);
    }

    #[test]
    fn only_a_fruit_crate_pays_and_only_a_player_is_paid() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        CrateBox::new(Kind::Weapon).linc(&mut driver, PLAYER, &mut rng);
        CrateBox::new(Kind::Time).linc(&mut driver, PLAYER, &mut rng);
        assert_eq!(
            (driver.pickup_wumpa_collected, rng),
            (0, math::RANDOM_NUMBER_INIT)
        );
        // A bot's hit breaks the crate and moves the RNG, and pays nothing.
        CrateBox::new(Kind::Fruit).linc(&mut driver, model_id::ROBOT_CAR, &mut rng);
        assert_eq!(driver.pickup_wumpa_collected, 0);
        assert_ne!(rng, math::RANDOM_NUMBER_INIT);
    }

    #[test]
    fn a_time_crate_stays_broken() {
        let mut driver = Driver::default();
        let mut rng = math::RANDOM_NUMBER_INIT;
        let mut c = CrateBox::new(Kind::Time);
        assert_eq!(c.linc(&mut driver, PLAYER, &mut rng), 1);
        tick_n(&mut c, 500);
        assert_eq!(c.scale, 0);
        // Touching it again does not bring it back either.
        c.linc(&mut driver, PLAYER, &mut rng);
        tick_n(&mut c, 500);
        assert_eq!(c.scale, 0);
    }
}
