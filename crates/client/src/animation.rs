//! The game tick, and the animation of the level's textures, scenery and water.

use bevy::prelude::*;
use render::mesh::{self, PARTS, to_world};

/// The game logic and its texture animation run at this rate; rendering may
/// run at any multiple of it.
pub(crate) const GAME_TICK_HZ: f64 = 30.0;

/// Animated level textures: which frame each shows, and the UVs to patch into
/// the part meshes when it changes.
#[derive(Resource)]
pub(crate) struct AnimState {
    pub(crate) animations: Vec<mesh::Animation>,
    pub(crate) faces: Vec<mesh::AnimatedFace>,
    pub(crate) uvs: Vec<Vec<[f32; 2]>>,
    pub(crate) handles: Vec<Option<Handle<Mesh>>>,
    pub(crate) shown: Vec<usize>,
}

/// Game ticks elapsed (`gGT->timer`).
#[derive(Resource)]
pub(crate) struct GameTick(pub(crate) u32);

/// The water faces: their meshes, and what the per-tick water animation patches into them.
#[derive(Resource)]
pub(crate) struct WaterState {
    pub(crate) water: loader::water::Water,
    pub(crate) corners: std::collections::HashMap<usize, Vec<mesh::WaterCorner>>,
    pub(crate) atlas_size: [f32; 2],
    pub(crate) colors: [Vec<[f32; 4]>; PARTS],
    pub(crate) uvs: [Vec<[f32; 2]>; PARTS],
    pub(crate) handles: [Option<Handle<Mesh>>; PARTS],
}

/// The swaying scenery (`AnimateQuad`): the vertices' mesh corners, and the meshes' arrays that
/// each tick patches.
#[derive(Resource)]
pub(crate) struct SceneryState {
    pub(crate) scenery: loader::scenery::Scenery,
    pub(crate) corners: std::collections::HashMap<usize, Vec<mesh::SceneryCorner>>,
    pub(crate) positions: Vec<Vec<[f32; 3]>>,
    pub(crate) colors: Vec<Vec<[f32; 4]>>,
    pub(crate) handles: Vec<Option<Handle<Mesh>>>,
}

/// `AnimateQuad`, once per game tick.
pub(crate) fn animate_scenery(
    mut state: ResMut<SceneryState>,
    tick: Res<GameTick>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let state = &mut *state;
    if state.scenery.vertices.is_empty() {
        return;
    }
    // Only the meshes whose arrays changed are uploaded again.
    let mut moved = vec![false; state.handles.len()];
    let mut recolored = vec![false; state.handles.len()];
    for update in state.scenery.animate(tick.0, |_| true) {
        let Some(corners) = state.corners.get(&update.vertex) else {
            continue;
        };
        for corner in corners {
            if let Some(pos) = update.pos {
                let at = &mut state.positions[corner.part][corner.slot];
                let pos = to_world(pos);
                moved[corner.part] |= std::mem::replace(at, pos) != pos;
            }
            if let Some([r, g, b, _]) = update.color {
                let [r, g, b] = [r, g, b].map(|c| mesh::vertex_channel(c as f32 * corner.gain));
                let at = &mut state.colors[corner.part][corner.slot];
                recolored[corner.part] |= std::mem::replace(at, [r, g, b, 1.0]) != [r, g, b, 1.0];
            }
        }
    }
    for part in 0..state.handles.len() {
        if !(moved[part] || recolored[part]) {
            continue;
        }
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            if moved[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, state.positions[part].clone());
            }
            if recolored[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, state.colors[part].clone());
            }
        }
    }
}

/// `CTR_CycleTex_LEV`, once per game tick.
pub(crate) fn animate(mut state: ResMut<AnimState>, tick: Res<GameTick>, mut meshes: ResMut<Assets<Mesh>>) {
    let state = &mut *state;
    let mut dirty = vec![false; state.handles.len()];
    for (i, anim) in state.animations.iter().enumerate() {
        let frame = anim.texture.frame_at(tick.0 as i32);
        if std::mem::replace(&mut state.shown[i], frame) == frame {
            continue;
        }
        for face in state.faces.iter().filter(|f| f.animation == i) {
            for (k, &from) in face.uv_order.iter().enumerate() {
                state.uvs[face.part][face.first_vertex + k] = anim.uvs[frame][from];
            }
            dirty[face.part] = true;
        }
    }
    for (part, _) in dirty.iter().enumerate().filter(|(_, d)| **d) {
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
        }
    }
}

/// `AnimateWater1P`, once per game tick, for every water vertex.
pub(crate) fn animate_water(
    mut state: ResMut<WaterState>,
    tick: Res<GameTick>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let state = &mut *state;
    let [width, height] = state.atlas_size;
    // Only the meshes whose arrays changed are uploaded again.
    let mut recolored = [false; PARTS];
    let mut moved = [false; PARTS];
    for c in state.water.animate(tick.0, |_| true) {
        // Textured primitives get their colour doubled, as everywhere else.
        let gray = mesh::vertex_channel(c.gray as f32 * 2.0 / 255.0);
        for corner in state.corners.get(&c.vertex).into_iter().flatten() {
            let color = [gray, gray, gray, 1.0];
            let uv = [
                (corner.origin[0] + c.uv[0] as f32) / width,
                (corner.origin[1] + c.uv[1] as f32) / height,
            ];
            recolored[corner.part] |=
                std::mem::replace(&mut state.colors[corner.part][corner.slot], color) != color;
            moved[corner.part] |=
                std::mem::replace(&mut state.uvs[corner.part][corner.slot], uv) != uv;
        }
    }
    for part in 0..PARTS {
        if !(recolored[part] || moved[part]) {
            continue;
        }
        if let Some(mut mesh) = state.handles[part].as_ref().and_then(|h| meshes.get_mut(h)) {
            if recolored[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, state.colors[part].clone());
            }
            if moved[part] {
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, state.uvs[part].clone());
            }
        }
    }
}

pub(crate) fn advance_tick(mut tick: ResMut<GameTick>) {
    tick.0 += 1;
}
