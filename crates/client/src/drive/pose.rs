//! Where the kart is drawn, from the sim's matrices and the model's animations.

use bevy::prelude::*;
use render::mesh::SCALE;
use vehicle::Driver;

/// Where the kart is drawn.
#[derive(Clone, Copy)]
pub(crate) struct Pose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

/// The rotation a game matrix stands for: its second column is up and its third forward. A matrix
/// that is flat (a squished kart) has none.
pub(crate) fn rotation_of(matrix: &vehicle::Matrix) -> Quat {
    let column = |c: usize| {
        Vec3::new(
            matrix.m[0][c] as f32,
            matrix.m[1][c] as f32,
            matrix.m[2][c] as f32,
        ) / 4096.0
    };
    let (up, forward) = (column(1), column(2));
    if up.length_squared() == 0.0 || forward.length_squared() == 0.0 {
        return Quat::IDENTITY;
    }
    let right = up.cross(forward).normalize_or_zero();
    let forward = right.cross(up.normalize_or_zero());
    Quat::from_mat3(&Mat3::from_cols(
        right,
        up.normalize_or_zero(),
        forward.normalize_or_zero(),
    ))
}

/// The pose of the model, from the matrix the sim gives the kart's instance (the
/// surface and heading, and the wheelie on top): its third column is forward and its
/// second up. The squash of the wheelie and the instance's own squash and stretch go in the scale.
pub(crate) fn pose_of(d: &Driver) -> Pose {
    let (matrix, t) = vehicle::baked::inst_matrix(d);
    let column = |c: usize| {
        Vec3::new(
            matrix.m[0][c] as f32,
            matrix.m[1][c] as f32,
            matrix.m[2][c] as f32,
        ) / 4096.0
    };
    let (side, up, forward) = (column(0), column(1), column(2));
    let rotation = rotation_of(&matrix);
    // The squash and stretch of the instance, on top of the wheelie's.
    let inst = Vec3::new(
        d.inst_scale_x as f32,
        d.inst_scale_y as f32,
        d.inst_scale_z as f32,
    ) / vehicle::force::KART_SCALE_BASE as f32;
    let basis = Vec3::new(side.length(), up.length(), forward.length());
    // A squished kart is flat: its height is 0, which a transform would not invert.
    let scale = basis * inst;
    Pose {
        translation: Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) * SCALE,
        rotation,
        scale: if basis.min_element() > 0.0 && inst.x > 0.0 && inst.z > 0.0 {
            scale.max(Vec3::splat(0.001))
        } else {
            Vec3::ONE
        },
    }
}

/// The pose of the animation at `frame` (`Instance::animFrame`). An animation that is
/// interpolated stores every other frame, and an odd frame is the average of its neighbours.
pub(crate) fn animation_pose(
    header: &loader::model::ModelHeader,
    index: u8,
    frame: i16,
) -> Option<loader::model::Frame> {
    let animation = header.animations.get(index as usize)?;
    let last = animation.frames.len().checked_sub(1)?;
    let frame = frame.max(0) as usize;
    if !animation.interpolated {
        return animation.frames.get(frame.min(last)).cloned();
    }
    let from = &animation.frames[(frame >> 1).min(last)];
    if frame & 1 == 0 {
        return Some(from.clone());
    }
    let to = &animation.frames[((frame >> 1) + 1).min(last)];
    Some(loader::model::Frame {
        origin: from.origin,
        vertices: from
            .vertices
            .iter()
            .zip(&to.vertices)
            .map(|(a, b)| std::array::from_fn(|i| (a[i] + b[i]) >> 1))
            .collect(),
    })
}
