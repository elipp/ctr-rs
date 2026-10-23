//! Wumpa fruit: what picking one up does to the driver. A fruit does not count at once: it joins
//! the queue of the pickup HUD (`PickupWumpaHUD`), which gives the driver one every six ticks
//! while the fruit flies to the counter (`UI_RenderFrame`), through `RB_Player_ModifyWumpa`.

use crate::{Driver, Effect, actions};

/// `DRIVER_WUMPA_MAX_COUNT`, and `DRIVER_WUMPA_JUICED_COUNT`: a full count is a juiced up kart.
const MAX_COUNT: i32 = 10;
const JUICED_COUNT: i32 = 10;
/// The ticks a fruit takes to fly to the counter: `PickupWumpaHUD.cooldown` of a pickup, and
/// what the next one waits.
pub const FLIGHT_TICKS: i16 = 5;
/// `OtherFX_Play` ids: a fruit arriving, and the kart getting juiced up.
const SOUND_ARRIVED: u8 = 0x42;
const SOUND_JUICED_UP: u8 = 0x41;

/// `RB_Player_ModifyWumpa` for a gain (use `spin::lose_wumpa` for losses). Returns whether the
/// kart was just juiced up (the HUD flashes).
pub fn gain_wumpa(d: &mut Driver, effects: &mut Vec<Effect>) -> bool {
    let before = d.num_wumpas as i32;
    d.num_wumpas = (before + 1).clamp(0, MAX_COUNT) as i8;
    let juiced = before < JUICED_COUNT && d.num_wumpas as i32 == JUICED_COUNT;
    if juiced {
        effects.push(Effect::Sound {
            id: SOUND_JUICED_UP,
            echo: false,
        });
    }
    juiced
}

/// A fruit picked up by a human driver (`RB_Fruit_ThCollide`): it starts flying to the counter.
pub fn pick_up(d: &mut Driver) {
    d.pickup_wumpa_cooldown = FLIGHT_TICKS;
    d.pickup_wumpa_collected += 1;
}

/// The queue of the pickup HUD, once per tick (`UI_RenderFrame`): when the last fruit has
/// arrived the driver gets it, and the next flies for [`FLIGHT_TICKS`]. A bot does not queue.
/// `racing` is `LOAD_IsOpen_RacingOrBattle() && !ADVENTURE_ARENA`: fruit of other menus count
/// for nothing.
pub fn hud_tick(d: &mut Driver, racing: bool, effects: &mut Vec<Effect>) {
    if d.pickup_wumpa_collected == 0 {
        return;
    }
    if d.pickup_wumpa_cooldown == 0 {
        d.pickup_wumpa_collected -= 1;
        if racing && d.actions_flag_set & actions::BOT == 0 {
            gain_wumpa(d, effects);
        }
        effects.push(Effect::Sound {
            id: SOUND_ARRIVED,
            echo: false,
        });
        if d.pickup_wumpa_collected != 0 {
            d.pickup_wumpa_cooldown = FLIGHT_TICKS;
        }
    } else {
        d.pickup_wumpa_cooldown -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fruit_arrive_one_every_six_ticks() {
        let mut d = Driver::default();
        let mut effects = Vec::new();
        for _ in 0..3 {
            pick_up(&mut d);
        }
        let mut counts = Vec::new();
        for _ in 0..20 {
            hud_tick(&mut d, true, &mut effects);
            counts.push(d.num_wumpas);
        }
        // Five ticks of flight, then one arrives; again after six.
        assert_eq!(counts[4], 0);
        assert_eq!(counts[5], 1);
        assert_eq!(counts[10], 1);
        assert_eq!(counts[11], 2);
        assert_eq!(counts[16], 2);
        assert_eq!(counts[17], 3);
        assert_eq!((d.pickup_wumpa_collected, d.pickup_wumpa_cooldown), (0, 0));
        assert_eq!(
            effects
                .iter()
                .filter(|e| matches!(
                    e,
                    Effect::Sound {
                        id: SOUND_ARRIVED,
                        ..
                    }
                ))
                .count(),
            3
        );
    }

    #[test]
    fn ten_is_juiced_up_and_the_most() {
        let mut d = Driver::default();
        let mut effects = Vec::new();
        d.num_wumpas = 9;
        assert!(gain_wumpa(&mut d, &mut effects));
        assert!(!gain_wumpa(&mut d, &mut effects));
        assert_eq!(d.num_wumpas, 10);
        assert_eq!(
            effects,
            [Effect::Sound {
                id: SOUND_JUICED_UP,
                echo: false
            }]
        );
    }

    #[test]
    fn fruit_outside_a_race_or_of_a_bot_count_for_nothing() {
        let mut d = Driver::default();
        pick_up(&mut d);
        for _ in 0..6 {
            hud_tick(&mut d, false, &mut Vec::new());
        }
        assert_eq!((d.num_wumpas, d.pickup_wumpa_collected), (0, 0));
    }
}
