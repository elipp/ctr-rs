//! The confetti of a race won (`RenderAllConfetti`): over our screen once the kart we play (or
//! watch) finished first, as retail puts it over the winner's. Where the pieces are is
//! `render::confetti`'s. It is a step of it for each tick of the race, two frames of retail's
//! counter (which counts the PlayStation's 60 Hz refreshes), and the pieces are drawn between the
//! last two, so they are as smooth as the camera. Drawn as one mesh of flat quads.
//!
//! The podium of a cup has them too, fewer (`PODIUM_CONFETTI_PARTICLE_COUNT`), from its own camera
//! (`podium.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use camera::CameraDc;
use loader::lev::SVec3;
use render::confetti::{Confetti, PARTICLES};
use render::mesh::SCALE;
use render::weather::Camera;

/// `PODIUM_CONFETTI_PARTICLE_COUNT`
const PODIUM_PARTICLES: i16 = 200;

/// `frameTimer_Confetti` counts the refreshes, two to a tick at 30 Hz.
const FRAMES_PER_TICK: u32 = 2;

/// A piece that moved farther than this (in world units) between two ticks wrapped round the box,
/// and is not drawn between the two places.
const WRAP_JUMP: f32 = 0x400 as f32 * SCALE;

/// How near the camera a corner can be, in world units: retail does not draw a piece with a
/// corner that the GTE cannot divide by (its `FLAG` bit 17: nearer than half the screen's
/// distance, 0x100, in front).
const NEAREST: f32 = 0x80 as f32 * SCALE;

/// A piece in the world: its corners and its colour (linear, for the vertex colours).
type Piece = ([Vec3; 4], [f32; 4]);

/// What the confetti of the race keeps: the game's state, its counter, and the last two ticks of
/// it.
#[derive(Default)]
pub struct ConfettiView {
    confetti: Confetti,
    timer: u32,
    pieces: [Vec<Piece>; 2],
}

impl ConfettiView {
    /// One tick of the race. `won` is for the kart over whose screen it is: it finished first
    /// (`PlayLevel_UpdateLapStats`). A win the race takes back (a rollback, a restart) takes its
    /// confetti with it.
    pub fn step(&mut self, camera: &CameraDc, won: bool) {
        if won {
            self.confetti.start();
        } else if self.confetti.count() != 0 {
            self.confetti = Confetti::default();
        }
        self.step_at(camera.pb_pos, camera.pb_rot.y);
    }

    /// The confetti of the podium (`CS_Podium_FullScene_Init`): `PODIUM_CONFETTI_PARTICLE_COUNT`
    /// pieces, from the start.
    pub fn podium() -> ConfettiView {
        let mut view = ConfettiView::default();
        view.confetti.start_with(PODIUM_PARTICLES);
        view
    }

    /// One tick, of a camera at `pos` that faces `yaw`.
    pub fn step_at(&mut self, pos: SVec3, yaw: i16) {
        self.confetti.step();
        self.timer = self.timer.wrapping_add(FRAMES_PER_TICK);
        let view = Camera { pos, yaw };
        let p = pos;
        let eye = Vec3::new(p.x as f32, p.y as f32, p.z as f32) * SCALE;
        self.pieces.swap(0, 1);
        self.pieces[1] = self
            .confetti
            .pieces(&view, self.timer)
            .into_iter()
            .map(|piece| {
                let corners = piece
                    .corners
                    .map(|[x, y, z]| eye + Vec3::new(x as f32, y as f32, z as f32) * SCALE);
                // The GPU puts the shade on the screen as it is.
                let [r, g, b] = piece.rgb;
                (corners, Color::srgb_u8(r, g, b).to_linear().to_f32_array())
            })
            .collect();
    }

    /// The race is paused: nothing moves between ticks.
    pub fn freeze(&mut self) {
        self.pieces[0] = self.pieces[1].clone();
    }

    /// The pieces `t` of the way from the last tick to this one.
    fn pieces(&self, t: f32) -> impl Iterator<Item = Piece> {
        self.pieces[1]
            .iter()
            .enumerate()
            .map(move |(i, &(now, color))| match self.pieces[0].get(i) {
                Some((before, _)) if before[0].distance(now[0]) < WRAP_JUMP => {
                    (std::array::from_fn(|k| before[k].lerp(now[k], t)), color)
                }
                _ => (now, color),
            })
    }
}

/// The mesh of the pieces.
#[derive(Component)]
pub struct ConfettiMesh;

/// Lays the pieces out in their mesh, which is put up the first time.
pub fn draw(
    mut commands: Commands,
    drive: Res<crate::drive::Drive>,
    fixed: Res<Time<Fixed>>,
    camera: Query<&Transform, With<crate::drive::ChaseCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut confetti: Query<(&Mesh3d, &mut Visibility), With<ConfettiMesh>>,
) {
    show(
        drive.confetti(),
        fixed.overstep_fraction(),
        camera.iter().next(),
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut confetti,
    );
}

/// Lays out `view`'s pieces `t` of the way from its last tick to this one in their mesh, which is
/// put up the first time. Those too near the `camera` that draws them are left out.
pub fn show(
    view: &ConfettiView,
    t: f32,
    camera: Option<&Transform>,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    confetti: &mut Query<(&Mesh3d, &mut Visibility), With<ConfettiMesh>>,
) {
    let quads = PARTICLES as usize;
    let Ok((mesh, mut visibility)) = confetti.single_mut() else {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4 * quads]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4 * quads]);
        mesh.insert_indices(Indices::U32(
            (0..quads as u32)
                .flat_map(|q| [0, 2, 1, 1, 2, 3].map(|i| q * 4 + i))
                .collect(),
        ));
        commands.spawn((
            ConfettiMesh,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
            NoFrustumCulling,
        ));
        return;
    };
    let shown = !view.pieces[1].is_empty();
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    if !shown {
        return;
    }
    let Some(mut mesh) = meshes.get_mut(&mesh.0) else {
        return;
    };
    let mut positions = vec![[0.0f32; 3]; 4 * quads];
    let mut colors = vec![[0.0f32; 4]; 4 * quads];
    let near = |corners: &[Vec3; 4]| {
        camera.is_some_and(|camera| {
            let forward = camera.forward();
            (corners.iter()).any(|&c| (c - camera.translation).dot(*forward) <= NEAREST)
        })
    };
    for (n, (corners, color)) in view
        .pieces(t)
        .filter(|(corners, _)| !near(corners))
        .take(quads)
        .enumerate()
    {
        for (k, corner) in corners.into_iter().enumerate() {
            positions[n * 4 + k] = corner.to_array();
            colors[n * 4 + k] = color;
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
}
