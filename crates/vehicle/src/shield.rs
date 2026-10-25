//! The shield bubble (`HELD_ITEM_SHIELD`): the driver holds it round the kart, and it absorbs one
//! hurt of any source (`VehPickState_NewState`), after which the kart is invincible for a moment.
//! The green one, which a driver without 10 wumpa fruit gets, runs out after 11520 ms and fades
//! over the last two seconds; the blue one (juiced) stays until it absorbs a hit.
//!
//! Like the mask ([`crate::stuck::MaskHead`]) it is a thread under the driver's: its state is
//! [`Shield`], in `Driver::shield`, and [`tick`] is `RB_ShieldDark_ThTick_Grow` and
//! `RB_ShieldDark_ThTick_Pop`. What it looks like (the scales, the alpha, the highlight that
//! sweeps round it) is kept in [`Shield`] for the outside to draw.
//!
//! The fire button shoots the bubble (`SHIELD_FLAG_SHOOT`, [`shoot`]): on its next tick it is gone
//! and the driver asks for a shield bomb (`Driver::shield_shot`), which the `weapon` crate flies.
//!
//! Not ported: the bubble's hit on a kart it bumps (`SHIELD_FLAG_CRASH_ATTACK` in
//! `VehPhysCrash_Attack`).

use crate::{Driver, Effect, actions, kart_state};

/// `SHIELD_DURATION_NORMAL`: milliseconds of a green shield.
const DURATION_NORMAL: i16 = 0x2d00;
/// What the duration shrinks by each tick (a literal in `RB_ShieldDark_ThTick_Grow`, not the
/// elapsed time), and where the fade starts.
const DURATION_STEP: i16 = 32;
const FADE_START: i16 = 1920;
/// `SHIELD_ALPHA_SCALE`: the `alphaScale` of a new shield. 0x1000 is gone.
const ALPHA_SCALE: u16 = 0x400;
/// `SHIELD_SCALE`
const SCALE: i16 = 0x700;
/// `SHIELD_HIGHLIGHT_ROT_Y`
const HIGHLIGHT_ROT_Y: i16 = 0xc00;
/// How long the highlight is away between its sweeps, and what it turns by each tick.
const HIGHLIGHT_PAUSE: i16 = 30;
const HIGHLIGHT_STEP: i16 = 0x100;

/// `VEH_PICK_SHIELD_DAMAGE_INVINCIBLE_TIMER`: milliseconds a kart is invincible after its shield
/// took a hit.
pub const DAMAGE_INVINCIBLE_TIMER: i32 = 0x2a0;

/// `DRIVER_WUMPA_JUICED_COUNT`
const WUMPA_JUICED_COUNT: i8 = 10;

const VOICELINE_SHIELD_SHOT: u8 = 13;
const VOICELINE_PRIORITY: u8 = 0x10;
const RUMBLE_FRAMES: u8 = 8;
const RUMBLE_FORCE: u8 = 0x7f;

const SOUND_SHIELD: u8 = 0x57;
const SOUND_POP: u8 = 0x58;

/// `R231.shieldGrowScale`, `shieldPopScale` and `shieldPulseScale`: the scale about the vertical
/// axis, and along it. 0x1000 is the model's size.
const GROW_SCALE: [[i16; 2]; 8] = [
    [977, 1835],
    [1792, 2936],
    [2205, 2095],
    [2335, 1254],
    [1884, 1612],
    [1433, 1971],
    [1612, 1881],
    [1792, 1792],
];
const POP_SCALE: [[i16; 2]; 11] = [
    [2150, 1612],
    [2419, 1433],
    [2508, 1344],
    [2329, 1523],
    [1792, 1792],
    [1254, 2150],
    [896, 2419],
    [716, 2508],
    [537, 2150],
    [358, 1254],
    [179, 537],
];
const PULSE_SCALE: [[i16; 2]; 6] = [
    [1845, 1756],
    [1899, 1720],
    [1971, 1684],
    [1899, 1720],
    [1845, 1756],
    [1792, 1792],
];

/// `struct Shield` and its thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shield {
    /// `SHIELD_FLAG_BLUE`: a juiced shield, which does not run out.
    pub blue: bool,
    /// `duration`, in milliseconds (a green shield).
    pub duration: i16,
    /// `instBubbleHold != NULL`: the driver holds it, and it takes the next hit.
    pub holding: bool,
    /// `SHIELD_FLAG_POP_ON_DAMAGE`: it took a hit, and pops on its next tick.
    pub pop_on_damage: bool,
    /// `SHIELD_FLAG_SHOOT`: the fire button was pressed, and it is shot on its next tick.
    pub shoot: bool,
    /// `animFrame` while it grows; while it pops, the frame of the pop (`None` before).
    pub grow_frame: i32,
    pub pop_frame: Option<i32>,
    /// `highlightRot.y` and `highlightTimer`, and whether the highlight shows.
    pub highlight_rot_y: i16,
    pub highlight_timer: i16,
    pub highlight_hidden: bool,
    /// `alphaScale` of the three instances: 0x1000 is gone.
    pub alpha_scale: u16,
    /// The scale of the bubble and its coloured layer, and of the highlight: about the vertical
    /// axis, and along it.
    pub scale: [i16; 2],
    pub highlight_scale: [i16; 2],
}

/// `VehPickupItem_ShootNow` for the shield item: the kart has a new shield, blue with 10 wumpa
/// fruit. (One that it had, which pops or runs out, is cut short.)
pub fn shoot_shield(d: &mut Driver, effects: &mut Vec<Effect>) {
    let blue = d.num_wumpas >= WUMPA_JUICED_COUNT;
    effects.push(Effect::Sound {
        id: SOUND_SHIELD,
        echo: d.actions_flag_set & actions::ENGINE_ECHO != 0,
    });
    d.shield = Some(Shield {
        blue,
        duration: if blue { 0 } else { DURATION_NORMAL },
        holding: true,
        pop_on_damage: false,
        shoot: false,
        grow_frame: 0,
        pop_frame: None,
        highlight_rot_y: HIGHLIGHT_ROT_Y,
        highlight_timer: 0,
        highlight_hidden: false,
        alpha_scale: ALPHA_SCALE,
        scale: [SCALE; 2],
        highlight_scale: [SCALE; 2],
    });
}

/// `VehPhysProc`'s "Shoot the bubble": the fire button, with a shield round the kart. False if
/// the kart has none.
pub fn shoot(d: &mut Driver) -> bool {
    let Some(shield) = d.shield.as_mut().filter(|s| s.holding) else {
        return false;
    };
    shield.shoot = true;
    shield.holding = false;
    true
}

/// What a hit does to a kart that holds a shield (`VehPickState_NewState`): the shield takes it
/// and pops on its next tick, and the kart is invincible for a moment. False if the kart has none.
pub fn absorb(d: &mut Driver) -> bool {
    let Some(shield) = d.shield.as_mut().filter(|s| s.holding) else {
        return false;
    };
    shield.pop_on_damage = true;
    shield.holding = false;
    d.invincible_timer = DAMAGE_INVINCIBLE_TIMER;
    true
}

/// `RB_ShieldDark_ThTick_Grow` and `RB_ShieldDark_ThTick_Pop`: the shield grows round the kart and
/// pulses, runs out (a green one), or pops, and is gone.
pub fn tick(d: &mut Driver, timer: u32, effects: &mut Vec<Effect>) {
    let Some(mut s) = d.shield else { return };

    if s.pop_frame.is_none() {
        // The highlight sweeps round, and rests between sweeps.
        if s.highlight_timer != 0 {
            s.highlight_timer -= 1;
            s.highlight_hidden = s.highlight_timer != 0;
        } else {
            s.highlight_rot_y = s.highlight_rot_y.wrapping_add(HIGHLIGHT_STEP);
            s.highlight_hidden = false;
            if s.highlight_rot_y % 0x1000 == 0x400 {
                s.highlight_timer = HIGHLIGHT_PAUSE;
                s.highlight_rot_y = HIGHLIGHT_ROT_Y;
                s.highlight_hidden = true;
            }
        }

        // It grows in eight ticks, then pulses; the highlight joins the pulse after.
        if s.grow_frame < GROW_SCALE.len() as i32 {
            s.scale = GROW_SCALE[s.grow_frame as usize];
            s.grow_frame += 1;
        } else {
            s.scale = PULSE_SCALE[(timer % 6) as usize];
            s.highlight_scale = s.scale;
        }

        if !s.blue {
            if s.duration == 0 {
                effects.push(Effect::Sound {
                    id: SOUND_POP,
                    echo: false,
                });
                d.shield = None;
                return;
            }
            s.duration -= DURATION_STEP;
            // The last two seconds, it fades.
            if s.duration < FADE_START {
                let faded = (60 - (s.duration >> 5) as i32) * 3072 / 60 + ALPHA_SCALE as i32;
                s.alpha_scale = faded as u16;
            }
        }

        if s.pop_on_damage
            || d.actions_flag_set & actions::RACE_FINISHED != 0
            || d.kart_state == kart_state::MASK_GRABBED
        {
            s.holding = false;
            s.pop_frame = Some(0);
        }
    }

    // Shot: the bubble is gone, and a shield bomb flies off in its place.
    if s.shoot && s.pop_frame.is_none() {
        effects.push(Effect::ShockFreq {
            driver_id: d.driver_id,
            a: RUMBLE_FRAMES,
            b: 0,
        });
        effects.push(Effect::ShockForce {
            driver_id: d.driver_id,
            a: RUMBLE_FRAMES,
            b: RUMBLE_FORCE,
        });
        if d.actions_flag_set & actions::BOT == 0 {
            effects.push(Effect::Voiceline {
                id: VOICELINE_SHIELD_SHOT,
                driver_id: d.driver_id,
                priority: VOICELINE_PRIORITY,
            });
        }
        effects.push(Effect::Sound {
            id: SOUND_POP,
            echo: false,
        });
        d.shield_shot = Some(s.blue);
        d.shield = None;
        return;
    }

    // (The pop starts in the tick that decides it.)
    if let Some(frame) = s.pop_frame {
        if (frame as usize) < POP_SCALE.len() {
            s.scale = POP_SCALE[frame as usize];
            s.pop_frame = Some(frame + 1);
        } else {
            effects.push(Effect::Sound {
                id: SOUND_POP,
                echo: false,
            });
            d.shield = None;
            return;
        }
    }
    d.shield = Some(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shielded(wumpas: i8) -> Driver {
        let mut d = Driver {
            kart_state: kart_state::NORMAL,
            num_wumpas: wumpas,
            ..Driver::default()
        };
        shoot_shield(&mut d, &mut Vec::new());
        d
    }

    fn run(d: &mut Driver, ticks: u32) {
        for t in 0..ticks {
            tick(d, t, &mut Vec::new());
        }
    }

    #[test]
    fn juiced_is_blue_and_the_rest_green() {
        assert!(shielded(10).shield.unwrap().blue);
        let green = shielded(9).shield.unwrap();
        assert!(!green.blue && green.duration == DURATION_NORMAL);
    }

    #[test]
    fn it_absorbs_one_hit_and_pops() {
        let mut d = shielded(0);
        run(&mut d, 3);
        assert!(absorb(&mut d));
        assert_eq!(d.invincible_timer, DAMAGE_INVINCIBLE_TIMER);
        // A second hit finds no shield (the invincible timer shrugs it off instead).
        assert!(!absorb(&mut d));
        // It pops over eleven ticks: the first runs in the tick that decides it.
        run(&mut d, POP_SCALE.len() as u32 - 1);
        assert!(d.shield.is_some());
        run(&mut d, 1);
        assert!(d.shield.is_none());
    }

    #[test]
    fn a_blue_shield_never_runs_out() {
        let mut d = shielded(10);
        run(&mut d, 5000);
        let s = d.shield.unwrap();
        assert!(s.holding && s.alpha_scale == ALPHA_SCALE);
    }

    #[test]
    fn a_green_shield_fades_over_two_seconds_and_goes() {
        let mut d = shielded(0);
        let ticks_to_fade = (DURATION_NORMAL - FADE_START) / DURATION_STEP;
        run(&mut d, ticks_to_fade as u32);
        assert_eq!(d.shield.unwrap().alpha_scale, ALPHA_SCALE);
        let mut last = ALPHA_SCALE;
        // 60 ticks of fade, always more transparent, then it goes with a sound.
        for t in 0..60 {
            tick(&mut d, t, &mut Vec::new());
            let alpha = d.shield.unwrap().alpha_scale;
            assert!(alpha > last);
            last = alpha;
        }
        let mut effects = Vec::new();
        tick(&mut d, 0, &mut effects);
        assert!(d.shield.is_none());
        assert_eq!(
            effects,
            [Effect::Sound {
                id: SOUND_POP,
                echo: false
            }]
        );
    }

    #[test]
    fn the_fire_button_shoots_it_on_the_next_tick() {
        for wumpas in [0, 10] {
            let mut d = shielded(wumpas);
            run(&mut d, 3);
            assert!(shoot(&mut d));
            // It takes no more hits, and cannot be shot again.
            assert!(!absorb(&mut d) && !shoot(&mut d));
            assert_eq!(d.shield_shot, None);
            tick(&mut d, 0, &mut Vec::new());
            assert!(d.shield.is_none());
            assert_eq!(d.shield_shot, Some(wumpas == 10));
        }
    }

    #[test]
    fn the_end_of_the_race_pops_it() {
        let mut d = shielded(10);
        d.actions_flag_set |= actions::RACE_FINISHED;
        run(&mut d, 1);
        assert!(!d.shield.unwrap().holding);
    }
}
