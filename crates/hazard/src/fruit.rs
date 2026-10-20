//! `RB_Fruit_*`: the wumpa fruit lying on the track. Touching one picks it up for good: it
//! disappears (scale 0, no thread) and joins the pickup HUD's queue ([`vehicle::wumpa`]). Players
//! and bots can take them, and nothing grows them back.

use coll::model_id;
use vehicle::{Driver, wumpa};

/// A wumpa fruit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fruit {
    /// Picked up: it is not drawn and has no hitbox any more (`scale.x == 0`).
    pub collected: bool,
}

impl Fruit {
    /// `RB_Fruit_LInC` (and `RB_Fruit_ThCollide`) for the `model_id` that touched it
    /// (`Input1.modelID`): 1 when it was picked up, 0 for anything that cannot.
    pub fn linc(&mut self, driver: &mut Driver, model: i16) -> i32 {
        if self.collected || !matches!(model, model_id::PLAYER | model_id::ROBOT_CAR) {
            return 0;
        }
        // Bots only erase the fruit; a player gets the HUD feedback.
        if model == model_id::PLAYER {
            wumpa::pick_up(driver);
        }
        self.collected = true;
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_takes_a_fruit_once() {
        let mut fruit = Fruit::default();
        let mut driver = Driver::default();
        assert_eq!(fruit.linc(&mut driver, model_id::PLAYER), 1);
        assert_eq!(driver.pickup_wumpa_collected, 1);
        assert!(fruit.collected);
        // Nothing left to take.
        assert_eq!(fruit.linc(&mut driver, model_id::PLAYER), 0);
        assert_eq!(driver.pickup_wumpa_collected, 1);
    }

    #[test]
    fn a_bot_only_erases_it_and_other_things_cannot_take_it() {
        let mut driver = Driver::default();
        let mut fruit = Fruit::default();
        assert_eq!(fruit.linc(&mut driver, 0x20), 0);
        assert!(!fruit.collected);
        assert_eq!(fruit.linc(&mut driver, model_id::ROBOT_CAR), 1);
        assert_eq!(driver.pickup_wumpa_collected, 0);
        assert!(fruit.collected);
    }
}
