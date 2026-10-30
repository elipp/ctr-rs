//! Which of the level's objects are reflected, and where: the ice under the crates of Blizzard
//! Bluff. The game decides it when the object is born (`RB_Default_LInB`,
//! `RB_MakeInstanceReflective`), from the quad block under it, and never again: the objects
//! stay where they are.

use coll::{Collision, search_flags};
use loader::lev::{InstDef, Level, SVec3, quad_flags};

use crate::pickup::crates::Kind;

/// How far above and below the object's place `RB_Default_LInB` looks for the ground.
const PROBE_ABOVE: i32 = 0x180;
const PROBE_BELOW: i32 = 0x80;

/// The height of the split line that `def` is reflected in (`Instance::vertSplit` with
/// `REFLECTIVE` set), if it is. Only the objects with `RB_Default_LInB` for their birth: the
/// crates and the wumpa fruit. (As if one player: the game reflects nothing in a split screen.)
pub fn split_line(level: &Level, def: &InstDef) -> Option<i16> {
    if Kind::of_model(def.model_id).is_none() && def.model_name != "fruit" {
        return None;
    }
    let mut coll = Collision::default();
    {
        let q = &mut coll.sps.quad_block_coll;
        q.quad_flags_wanted = (quad_flags::GROUND | quad_flags::COLLISION_SURFACE) as u32;
        q.quad_flags_ignored = 0;
        q.search_flags = search_flags::HIGH_LOD;
    }
    // (The game truncates each coordinate to a halfword once, for both ends.)
    let at = |y: i32| SVec3 {
        x: def.pos.x,
        y: y as i16,
        z: def.pos.z,
    };
    let y = def.pos.y as i32;
    coll.search_quadblocks(level, at(y - PROBE_ABOVE), at(y + PROBE_BELOW), 0);
    if coll.sps.did_touch_quadblock == 0 || coll.sps.did_touch_hitbox != 0 {
        return None;
    }
    let quad = level.quad_blocks.get(coll.sps.hit.quad? as usize)?;
    if quad.flags & quad_flags::COLLISION_SURFACE != 0 {
        return None;
    }
    if quad.flags & quad_flags::REFLECT_SPLIT_LINE_1 != 0 {
        Some(level.split_lines[1])
    } else if quad.flags & quad_flags::REFLECT_SPLIT_LINE_0 != 0 {
        Some(level.split_lines[0])
    } else {
        None
    }
}
