//! The sim-side half of `game/Vehicle/VehGroundShadow.c`: whether a driver's
//! ground shadow is drawn and how it is oriented and scaled. The quads and
//! their textures are the renderer's (`render::shadow`).

use gte::Gte;

use crate::force::rot_axis_angle;
use crate::{Driver, Matrix};

/// A driver's shadow on the ground it last touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundShadow {
    /// Game units: the driver's x and z, and the ground's height plus 3.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// 1..=0x100: 0x100 on the ground, shrinking as the driver rises, which
    /// scales the shadow.
    pub height: i32,
    /// `VehPhysForce_RotAxisAngle` of the ground's normal and the driver's
    /// heading: columns are right, up (the normal) and forward, 0x1000 = 1.0.
    pub axes: Matrix,
}

/// The shadow `VehGroundShadow_Main` would draw for `d`, if any: none once the
/// driver is at least 0x100 units above the ground. `gte` is scratch.
pub fn ground_shadow(d: &Driver, gte: &mut Gte) -> Option<GroundShadow> {
    let above_ground = (d.pos_curr.y.wrapping_sub(d.quad_block_height)) >> 8;
    let height = 0x100 - above_ground;
    if !(1..0x10a).contains(&height) {
        return None;
    }
    let mut axes = Matrix::default();
    rot_axis_angle(&mut axes, gte, d.axis_angle3_normal_vec, d.rot_curr_y as i32);
    Some(GroundShadow {
        x: d.pos_curr.x >> 8,
        y: (d.quad_block_height >> 8) + 3,
        z: d.pos_curr.z >> 8,
        height: height.min(0x100),
        axes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver_at(above_ground: i32) -> Driver {
        let mut d = Driver::new(0, 0);
        d.quad_block_height = 100 << 8;
        d.pos_curr.y = (100 + above_ground) << 8;
        d.axis_angle3_normal_vec = loader::lev::SVec3 { x: 0, y: 0x1000, z: 0 };
        d
    }

    #[test]
    fn shadow_shrinks_with_height_and_goes_out_of_reach() {
        let mut gte = Gte::default();
        let on_ground = ground_shadow(&driver_at(0), &mut gte).unwrap();
        assert_eq!((on_ground.height, on_ground.y), (0x100, 103));
        assert_eq!(ground_shadow(&driver_at(100), &mut gte).unwrap().height, 156);
        // Slightly below the ground still counts as on it; far above is no shadow.
        assert_eq!(ground_shadow(&driver_at(-5), &mut gte).unwrap().height, 0x100);
        assert_eq!(ground_shadow(&driver_at(255), &mut gte).unwrap().height, 1);
        assert!(ground_shadow(&driver_at(256), &mut gte).is_none());
        assert!(ground_shadow(&driver_at(300), &mut gte).is_none());
        assert!(ground_shadow(&driver_at(-20), &mut gte).is_none());
        // Flat ground: up is the normal, forward lies in the plane.
        let m = on_ground.axes.m;
        assert_eq!([m[0][1], m[1][1], m[2][1]], [0, 0x1000, 0]);
        assert_eq!(m[1][2], 0);
    }
}
