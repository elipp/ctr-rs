//! The rain of the red beaker's cloud (`RedBeaker_RenderRain`): short streaks that fall in a box
//! under the cloud. The game draws them as lines of a PlayStation's GPU, 30 of them (`frameCount`)
//! scrolling down 0x28 units a frame, in places a small pseudo-random generator picks. This is
//! where they are, in game units from the cloud; the viewer draws them.

const CENTER_XY: u32 = 0x0200_0080;
const XY_MASK: u32 = 0xfffe_ffff;
const WRAP_MASK: u32 = 0x01fe_00ff;

/// `RainLocal::vel.y`: how far the rain falls each frame.
const FALL: i16 = -0x28;

/// `RedBeaker_NextRngByte`
fn next_byte(state0: &mut u32, state1: &mut u32) -> u32 {
    let shifted = *state1 >> 8;
    let mut mixed = state0.wrapping_add(shifted);
    *state0 = (*state0 >> 8) | (*state1 << 24);
    mixed = mixed.wrapping_add(*state0 >> 8);
    mixed <<= 24;
    *state1 = shifted | mixed;
    mixed
}

/// `RedBeaker_NextRng`: the packed `x`, `y` and the `z` of the next streak.
fn next(state0: &mut u32, state1: &mut u32) -> (u32, i32) {
    let x = next_byte(state0, state1);
    let y = next_byte(state0, state1);
    let z = next_byte(state0, state1);
    let xy = (((x as i32) >> 24) as u32 & 0xffff) | (((y as i32) >> 7) as u32);
    (xy, (z as i32) >> 23)
}

fn point(xy: u32, z: u32) -> [i32; 3] {
    [
        (xy & 0xffff) as i16 as i32,
        (xy >> 16) as i16 as i32,
        z as i32 - 0x80,
    ]
}

/// `RedBeaker_RenderPass`: the streaks of the cloud's rain `age` frames after it began, with
/// `frames` of them (`RainLocal::frameCount`), as the two ends of each in game units from the
/// cloud. The upper end is the one that is dimmer.
pub fn streaks(frames: i16, age: u32) -> Vec<[[i32; 3]; 2]> {
    // The scroll is packed as the game keeps it: x in the low half, y in the high half, and only
    // the y moves.
    let velocity = ((FALL as u16 as u32) << 16) & XY_MASK;
    let scroll = velocity.wrapping_mul(age);
    let next_scroll = scroll.wrapping_add(velocity) & XY_MASK;

    let (mut state0, mut state1) = (0x3012_5400_u32, 0x4935_83fe_u32);
    let (mut xy, mut z) = next(&mut state0, &mut state1);
    let mut out = Vec::new();
    for _ in 0..frames.max(0) {
        let xy0 = (xy.wrapping_add(scroll) & WRAP_MASK).wrapping_sub(CENTER_XY);
        let xy1 = (xy.wrapping_add(next_scroll) & WRAP_MASK).wrapping_sub(CENTER_XY);
        // A streak that wrapped round the box this frame is skipped.
        if xy1.wrapping_sub(xy0) == velocity {
            let z0 = z as u32 & 0xff;
            // (z does not scroll: both ends are at the same depth.)
            out.push([point(xy0 & XY_MASK, z0), point(xy1 & XY_MASK, z0)]);
            (xy, z) = next(&mut state0, &mut state1);
        }
        (xy, z) = next(&mut state0, &mut state1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rain_falls_forty_units_a_streak_inside_the_box() {
        for age in [0, 1, 7, 100, 5000] {
            let rain = streaks(30, age);
            assert!(rain.len() > 20, "{} at {age}", rain.len());
            for [top, bottom] in rain {
                assert_eq!(top[1] - bottom[1], 0x28);
                assert_eq!(top[0], bottom[0]);
                assert!((-128..128).contains(&top[0]) && (-128..128).contains(&top[2]));
                assert!((-512..0).contains(&top[1]));
            }
        }
    }

    #[test]
    fn no_frames_no_rain() {
        assert!(streaks(0, 3).is_empty());
        assert!(streaks(-2, 3).is_empty());
    }
}
