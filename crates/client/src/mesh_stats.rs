//! `CTR_MESH_STATS=1`: what makes the render world upload meshes again. Every mesh that is added,
//! modified or removed in a frame is extracted anew, and `allocate_and_free_meshes` frees and
//! allocates it in the mesh slabs and copies its data, so its cost goes with these events.
//!
//! Every [`WINDOW`] this logs, per frame on average, the events of each kind of mesh and the bytes
//! of vertex and index data they upload. A mesh's kind is the components of the entity that draws
//! it (and of that entity's parent), less Bevy's own. Their names need Bevy's `debug` feature
//! (`--features debug`, or `dev`); without it all of them read the same.

use std::collections::HashMap;
use std::time::Duration;

use bevy::asset::AssetEvent;
use bevy::mesh::{Indices, Mesh, Mesh3d};
use bevy::prelude::*;

/// How long the events are summed before they are logged.
const WINDOW: Duration = Duration::from_secs(2);

/// The most kinds of mesh logged, the busiest first.
const TOP: usize = 15;

/// Whether `CTR_MESH_STATS=1` is set.
pub fn enabled() -> bool {
    std::env::var("CTR_MESH_STATS").as_deref() == Ok("1")
}

pub struct MeshStatsPlugin;

impl Plugin for MeshStatsPlugin {
    fn build(&self, app: &mut App) {
        if !enabled() {
            return;
        }
        if !cfg!(any(feature = "debug", feature = "dev")) {
            warn!("CTR_MESH_STATS: without the `debug` feature the component names are not known");
        }
        app.init_resource::<MeshStats>()
            .add_systems(Last, (collect, tally, report).chain());
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Added,
    Modified,
    Removed,
}

/// What a kind of mesh did over the window.
#[derive(Default)]
struct Tally {
    added: u64,
    modified: u64,
    removed: u64,
    /// The vertex and index bytes of the added and modified ones.
    bytes: u64,
    /// The meshes of this kind that had any event over the window (not per frame).
    meshes: std::collections::HashSet<AssetId<Mesh>>,
}

#[derive(Resource)]
struct MeshStats {
    /// This frame's events, to be put to a kind by `tally`.
    frame: Vec<(AssetId<Mesh>, Kind, u64)>,
    tallies: HashMap<String, Tally>,
    frames: u64,
    since: Duration,
}

impl Default for MeshStats {
    fn default() -> Self {
        MeshStats {
            frame: Vec::new(),
            tallies: HashMap::new(),
            frames: 0,
            since: Duration::ZERO,
        }
    }
}

/// The bytes of a mesh's vertex and index data, as the allocator uploads them; 0 once the
/// data has gone to the render world.
fn bytes_of(mesh: &Mesh) -> u64 {
    let vertices: usize = mesh
        .try_attributes()
        .map(|attributes| attributes.map(|(_, values)| values.get_bytes().len()).sum())
        .unwrap_or(0);
    let indices = match mesh.try_indices_option() {
        Ok(Some(Indices::U16(i))) => i.len() * 2,
        Ok(Some(Indices::U32(i))) => i.len() * 4,
        _ => 0,
    };
    (vertices + indices) as u64
}

/// This frame's mesh events.
fn collect(
    mut events: MessageReader<AssetEvent<Mesh>>,
    meshes: Res<Assets<Mesh>>,
    mut stats: ResMut<MeshStats>,
) {
    for event in events.read() {
        let (id, kind) = match *event {
            AssetEvent::Added { id } => (id, Kind::Added),
            AssetEvent::Modified { id } => (id, Kind::Modified),
            AssetEvent::Removed { id } => (id, Kind::Removed),
            AssetEvent::Unused { .. } | AssetEvent::LoadedWithDependencies { .. } => continue,
        };
        let bytes = match kind {
            Kind::Removed => 0,
            _ => meshes.get(id).map_or(0, bytes_of),
        };
        stats.frame.push((id, kind, bytes));
    }
}

/// The components of `entity` that are not Bevy's, by their short names, sorted.
fn label_of(world: &World, entity: Entity) -> String {
    let Ok(components) = world.inspect_entity(entity) else {
        return "?".into();
    };
    let mut names: Vec<String> = components
        .filter_map(|(_, info)| {
            let name = info.name();
            let full = format!("{name}");
            // (Bevy's, and the std's wrappers of them.)
            if full.starts_with("bevy_") || full.starts_with("core::") {
                return None;
            }
            Some(format!("{}", name.shortname()))
        })
        .collect();
    names.sort();
    if names.is_empty() {
        "-".into()
    } else {
        names.join("+")
    }
}

/// Puts this frame's events to the kind of the entity that draws each mesh.
fn tally(world: &mut World) {
    let frame = std::mem::take(&mut world.resource_mut::<MeshStats>().frame);
    if frame.is_empty() {
        world.resource_mut::<MeshStats>().frames += 1;
        return;
    }
    let mut drawn_by: HashMap<AssetId<Mesh>, Entity> = HashMap::new();
    let mut query = world.query::<(Entity, &Mesh3d)>();
    for (entity, mesh) in query.iter(world) {
        drawn_by.entry(mesh.id()).or_insert(entity);
    }
    let mut labels: HashMap<Entity, String> = HashMap::new();
    let mut resolved = Vec::with_capacity(frame.len());
    for (id, kind, bytes) in frame {
        let label = match drawn_by.get(&id) {
            None => match kind {
                Kind::Removed => "(removed)".to_string(),
                _ => "(no Mesh3d)".to_string(),
            },
            Some(&entity) => labels
                .entry(entity)
                .or_insert_with(|| {
                    let own = label_of(world, entity);
                    match world.get::<ChildOf>(entity) {
                        Some(parent) => format!("{} > {own}", label_of(world, parent.parent())),
                        None => own,
                    }
                })
                .clone(),
        };
        resolved.push((id, kind, bytes, label));
    }
    let mut stats = world.resource_mut::<MeshStats>();
    stats.frames += 1;
    for (id, kind, bytes, label) in resolved {
        let tally = stats.tallies.entry(label).or_default();
        match kind {
            Kind::Added => tally.added += 1,
            Kind::Modified => tally.modified += 1,
            Kind::Removed => tally.removed += 1,
        }
        tally.bytes += bytes;
        tally.meshes.insert(id);
    }
}

/// Logs the window's tallies, per frame, and starts the next window.
fn report(time: Res<Time<Real>>, mut stats: ResMut<MeshStats>) {
    let now = time.elapsed();
    if now - stats.since < WINDOW {
        return;
    }
    let frames = stats.frames.max(1) as f64;
    let mut rows: Vec<(&String, &Tally)> = stats.tallies.iter().collect();
    let events = |t: &Tally| t.added + t.modified + t.removed;
    rows.sort_by_key(|(_, t)| std::cmp::Reverse(events(t)));
    let total_events: u64 = rows.iter().map(|(_, t)| events(t)).sum();
    let total_bytes: u64 = rows.iter().map(|(_, t)| t.bytes).sum();
    let mut text = format!(
        "mesh events over {:.0} frames, per frame: {:.1} events, {:.1} KiB\n  {:>6} {:>6} {:>6} {:>7} {:>9}  kind",
        frames,
        total_events as f64 / frames,
        total_bytes as f64 / frames / 1024.0,
        "added",
        "modif",
        "remov",
        "meshes",
        "KiB",
    );
    for (label, t) in rows.iter().take(TOP) {
        text.push_str(&format!(
            "\n  {:>6.1} {:>6.1} {:>6.1} {:>7} {:>9.1}  {label}",
            t.added as f64 / frames,
            t.modified as f64 / frames,
            t.removed as f64 / frames,
            t.meshes.len(),
            t.bytes as f64 / frames / 1024.0,
        ));
    }
    if rows.len() > TOP {
        text.push_str(&format!("\n  ... and {} more kinds", rows.len() - TOP));
    }
    info!("{text}");
    stats.tallies.clear();
    stats.frames = 0;
    stats.since = now;
}
