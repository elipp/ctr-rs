//! The picture of a track for `/track` to show: the game's own for its racing tracks. A map has
//! none, so we take one, which is kept by the map's hash, in one of two ways:
//!
//! - When `/track` has offered a map for a moment, its level alone (no objects, no karts) is built
//!   away from the scene, on layers no other camera draws, for one picture. This is the preview.
//! - The first time a map is driven on, cameras of our own take a picture of the scene from the
//!   chase camera, which replaces a preview.
//!
//! Until then, or if the disc's pictures cannot be read, it is the track's minimap.

use std::collections::{HashMap, HashSet};
use std::io;

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use loader::dram::DramFile;
use loader::icons::parse_level_icons;
use loader::lev::Minimap;
use loader::source::Need;
use loader::vram::{Vram, parse_vram_file};
use render::mesh::{self, Atlas, LevelMesh};
use render::minimap::map_image;
use render::track_thumbnails::{self, HEIGHT, ICON_IDS, WIDTH};

use crate::assets::upload_shared;
use crate::drive::{ChaseCamera, Kart};
use crate::kart::KartSlot;
use crate::leaderboard::atlas_image;
use crate::loading::Loading;
use crate::lobby::Lobby;
use crate::scene::{spawn_layers, spawn_parts};
use crate::shadow::Shadow;
use crate::sky::{self, SkyCamera};
use crate::subtract::SubtractMaterial;
use crate::{LevelNumber, OverlayCamera, maps};

/// How many pixels of the screen a texel of the disc's pictures, and of a minimap, takes.
const PICTURE_SCALE: f32 = 2.0;

/// The size of a picture we take: that of the disc's on the screen.
const CAPTURE_SIZE: UVec2 = UVec2::new(
    WIDTH as u32 * PICTURE_SCALE as u32,
    HEIGHT as u32 * PICTURE_SCALE as u32,
);

/// The frames the cameras draw before the picture is taken: what was spawned with them, and the
/// karts that were hidden, have come through to the renderer by then.
const SETTLE_FRAMES: u32 = 2;

/// A layer no camera draws: where the karts' shadows are while a picture is taken, as they set
/// their own visibility every frame.
const HIDDEN_LAYER: usize = 31;

/// The layers of a preview, which no other camera draws.
const PREVIEW_LAYER: usize = 30;
const PREVIEW_SKY_LAYER: usize = 29;

/// How long `/track` has to offer a map before its preview is built, in seconds: the arrows go
/// through the others without building each.
const PREVIEW_DELAY: f32 = 0.3;

/// The frames a preview's cameras draw before the picture is taken, and how many times it is taken
/// again if it comes out empty: a whole level takes the renderer longer to take in.
const PREVIEW_SETTLE_FRAMES: u32 = 8;
const PREVIEW_TRIES: u32 = 5;

/// The most frames to wait for a preview's picture to come back.
const PREVIEW_GIVE_UP_FRAMES: u32 = 600;

/// Less of a preview's picture than this is lit: the renderer was not ready, so it is taken again.
const PREVIEW_MIN_LIT: f32 = 0.05;

/// Where a preview looks from and at, from the first place on the grid.
const PREVIEW_BEHIND: f32 = 4.0;
const PREVIEW_ABOVE: f32 = 1.6;
const PREVIEW_AHEAD: f32 = 8.0;
const PREVIEW_LOOK_ABOVE: f32 = 0.8;

#[derive(Clone, PartialEq)]
pub(crate) struct Picture {
    pub image: Handle<Image>,
    pub rect: Rect,
    /// On the screen.
    pub size: Vec2,
}

#[derive(Clone)]
struct CapturedPicture {
    picture: Picture,
    /// Of the level alone: the first drive on the map takes a whole one.
    preview: bool,
}

#[derive(Resource, Default)]
pub(crate) struct TrackPictures {
    /// Once tried: none if they cannot be read.
    thumbnails: Option<Option<Handle<Image>>>,
    minimaps: HashMap<u32, Option<Picture>>,
    /// Those we took, by the map's hash: none if there is none yet.
    captured: HashMap<u64, Option<CapturedPicture>>,
    /// The map `/track` offers and we have no picture of, and since when.
    awaiting_preview: Option<(u32, f32)>,
    /// The maps whose preview has been tried.
    previewed: HashSet<u64>,
}

impl TrackPictures {
    pub fn of(&mut self, id: u32, images: &mut Assets<Image>) -> Option<Picture> {
        self.thumbnail(id, images)
            .or_else(|| Some(self.captured(id, images)?.picture))
            .or_else(|| self.minimap(id, images))
    }

    fn thumbnail(&mut self, id: u32, images: &mut Assets<Image>) -> Option<Picture> {
        let row = id as usize;
        if row >= ICON_IDS.len() {
            return None;
        }
        let image = self
            .thumbnails
            .get_or_insert_with(|| match thumbnails() {
                Ok(atlas) => Some(atlas_image(atlas, images)),
                Err(why) => {
                    warn!("no pictures of the tracks: {why}");
                    None
                }
            })
            .clone()?;
        Some(Picture {
            image,
            rect: Rect::new(
                0.0,
                (row * HEIGHT) as f32,
                WIDTH as f32,
                ((row + 1) * HEIGHT) as f32,
            ),
            size: Vec2::new(WIDTH as f32, HEIGHT as f32) * PICTURE_SCALE,
        })
    }

    /// What `/track` offers now, for the preview of a map we have no picture of.
    pub fn set_offered(&mut self, id: Option<u32>, now: f32, images: &mut Assets<Image>) {
        let id = id
            .filter(|&id| map::is_map_id(id))
            .filter(|&id| self.captured(id, images).is_none());
        if self
            .awaiting_preview
            .map(|(awaiting_preview, _)| awaiting_preview)
            != id
        {
            self.awaiting_preview = id.map(|id| (id, now));
        }
    }

    fn captured(&mut self, id: u32, images: &mut Assets<Image>) -> Option<CapturedPicture> {
        let hash = maps::hash(id)?;
        self.captured
            .entry(hash)
            .or_insert_with(|| {
                let (size, preview, rgba) = decode(&store::load(hash)?)?;
                Some(CapturedPicture {
                    picture: captured_picture(size, rgba, images),
                    preview,
                })
            })
            .clone()
    }

    fn minimap(&mut self, id: u32, images: &mut Assets<Image>) -> Option<Picture> {
        self.minimaps
            .entry(id)
            .or_insert_with(|| {
                let atlas = minimap(id)?;
                let rect = Rect::new(0.0, 0.0, atlas.width as f32, atlas.height as f32);
                Some(Picture {
                    image: atlas_image(atlas, images),
                    rect,
                    size: rect.size() * PICTURE_SCALE,
                })
            })
            .clone()
    }
}

fn thumbnails() -> io::Result<Atlas> {
    let source = crate::disc::source();
    let mut vram = Vram::default();
    for need in [Need::MenuVram, Need::SharedVram] {
        for image in parse_vram_file(&source.get(need)?)? {
            vram.upload(&image)?;
        }
    }
    let lev = DramFile::parse(&source.get(Need::MenuLev)?)?;
    Ok(track_thumbnails::atlas(&parse_level_icons(&lev)?, &vram))
}

/// The minimap of the level, opaque: its panel is black, which is what retail's additive texels
/// come to over black.
fn minimap(id: u32) -> Option<Atlas> {
    let map = maps::get(id)?;
    let mut vram = Vram::default();
    for image in &map.vram {
        vram.upload(image).ok()?;
    }
    upload_shared(&mut vram, &crate::disc::source());
    let layout = Minimap::parse(&map.dram).ok()??;
    let icons = parse_level_icons(&map.dram).ok()?;
    let mut atlas = map_image(&icons, &vram, layout.top_half_mode == 0)?;
    for texel in atlas.rgba.chunks_mut(4) {
        texel[3] = 255;
    }
    Some(atlas)
}

fn captured_picture(size: UVec2, rgba: Vec<u8>, images: &mut Assets<Image>) -> Picture {
    let atlas = Atlas {
        width: size.x as usize,
        height: size.y as usize,
        rgba,
    };
    let rect = Rect::new(0.0, 0.0, size.x as f32, size.y as f32);
    Picture {
        image: atlas_image(atlas, images),
        rect,
        size: rect.size(),
    }
}

/// A picture as it is kept: the map's hash, the size, whether it is a preview, and the texels.
fn encode(hash: u64, size: UVec2, preview: bool, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(20 + rgba.len());
    bytes.extend_from_slice(&hash.to_le_bytes());
    bytes.extend_from_slice(&size.x.to_le_bytes());
    bytes.extend_from_slice(&size.y.to_le_bytes());
    bytes.extend_from_slice(&(preview as u32).to_le_bytes());
    bytes.extend_from_slice(rgba);
    bytes
}

/// The size, whether it is a preview, and the texels of what [`encode`] made; none if it is
/// something else.
fn decode(bytes: &[u8]) -> Option<(UVec2, bool, Vec<u8>)> {
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let size = UVec2::new(word(8)?, word(12)?);
    let preview = word(16)? != 0;
    let rgba = bytes.get(20..)?;
    (rgba.len() == size.x as usize * size.y as usize * 4).then(|| (size, preview, rgba.to_vec()))
}

/// Where the pictures are kept: a file each.
#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::path::PathBuf;

    fn path(hash: u64) -> PathBuf {
        crate::paths::track_pictures().join(format!("{hash:016x}.rgba"))
    }

    pub fn load(hash: u64) -> Option<Vec<u8>> {
        std::fs::read(path(hash)).ok()
    }

    pub fn save(hash: u64, bytes: &[u8]) {
        let result = std::fs::create_dir_all(crate::paths::track_pictures())
            .and_then(|()| std::fs::write(path(hash), bytes));
        if let Err(why) = result {
            bevy::log::warn!("could not keep the picture of the track: {why}");
        }
    }
}

/// Where the pictures are kept: the page's storage, which gives them all at the start.
#[cfg(target_arch = "wasm32")]
mod store {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    static SAVED: OnceLock<Mutex<HashMap<u64, Vec<u8>>>> = OnceLock::new();

    fn saved() -> &'static Mutex<HashMap<u64, Vec<u8>>> {
        SAVED.get_or_init(|| {
            let saved = crate::web::take_track_pictures()
                .into_iter()
                .filter_map(|bytes| {
                    let hash = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?);
                    Some((hash, bytes))
                });
            Mutex::new(saved.collect())
        })
    }

    pub fn load(hash: u64) -> Option<Vec<u8>> {
        saved().lock().ok()?.get(&hash).cloned()
    }

    pub fn save(hash: u64, bytes: &[u8]) {
        crate::web::save_track_picture(&format!("{hash:016x}"), bytes);
    }
}

#[derive(Resource)]
struct Capture {
    hash: u64,
    target: Handle<Image>,
    cameras: Vec<Entity>,
    source: CaptureSource,
    frames: u32,
    /// How many pictures of a preview came out empty.
    tries: u32,
    /// The picture has come back.
    done: bool,
}

/// What a picture is taken of.
enum CaptureSource {
    /// The scene of this level, with the karts hidden, and how they were, and their shadows.
    Scene {
        level: u32,
        hidden: Vec<(Entity, Visibility)>,
        shadows: Vec<Entity>,
    },
    /// A level built for it, under this entity.
    Preview(Entity),
}

impl Capture {
    fn settle_frames(&self) -> u32 {
        match self.source {
            CaptureSource::Scene { .. } => SETTLE_FRAMES,
            CaptureSource::Preview(_) => PREVIEW_SETTLE_FRAMES,
        }
    }
}

fn capture_target(images: &mut Assets<Image>) -> Handle<Image> {
    images.add(Image::new_target_texture(
        CAPTURE_SIZE.x,
        CAPTURE_SIZE.y,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ))
}

/// A camera that takes the picture: not "the camera" that the wheels, the smoke and the rest follow
/// (`OverlayCamera`), and not on the window, or the HUD would be drawn for it.
fn camera(order: isize, clear_color: ClearColorConfig, target: &Handle<Image>) -> impl Bundle {
    (
        OverlayCamera,
        Camera3d::default(),
        Camera {
            order,
            clear_color,
            ..default()
        },
        RenderTarget::Image(target.clone().into()),
    )
}

/// Once a scene is whole and we have no picture of its map, or only a preview, draws it from the
/// chase camera, without the karts. (The screen is still black then: the hidden karts are not seen
/// to go.)
fn start_capture(
    level: Res<LevelNumber>,
    loading: Option<Res<Loading>>,
    taking: Option<Res<Capture>>,
    chase: Query<(&Camera, &Projection, &GlobalTransform), With<ChaseCamera>>,
    sky: Query<(&Camera, &Projection, &RenderLayers), With<SkyCamera>>,
    mut karts: Query<(Entity, &mut Visibility), Or<(With<Kart>, With<KartSlot>)>>,
    shadows: Query<Entity, With<Shadow>>,
    mut pictures: ResMut<TrackPictures>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut tried: Local<Option<u32>>,
) {
    if loading.is_some() || taking.is_some() || *tried == Some(level.0) {
        return;
    }
    // (It only goes behind the kart once the clock runs again.)
    let Some((main, projection, at)) = chase
        .single()
        .ok()
        .filter(|(_, _, at)| at.translation() != Vec3::ZERO)
    else {
        return;
    };
    *tried = Some(level.0);
    let Some(hash) = maps::hash(level.0).filter(|_| map::is_map_id(level.0)) else {
        return;
    };
    if pictures
        .captured(level.0, &mut images)
        .is_some_and(|captured| !captured.preview)
    {
        return;
    }
    let target = capture_target(&mut images);
    let mut cameras = vec![
        commands
            .spawn((
                camera(-10, main.clear_color.clone(), &target),
                projection.clone(),
                at.compute_transform(),
            ))
            .id(),
    ];
    cameras.extend(sky.iter().next().map(|(sky, projection, layers)| {
        commands
            .spawn((
                camera(-11, sky.clear_color.clone(), &target),
                projection.clone(),
                layers.clone(),
                Transform::from_rotation(at.rotation()),
            ))
            .id()
    }));
    let hidden = karts
        .iter_mut()
        .map(|(kart, mut visibility)| {
            (
                kart,
                std::mem::replace(&mut *visibility, Visibility::Hidden),
            )
        })
        .collect();
    let shadows: Vec<Entity> = shadows.iter().collect();
    for shadow in &shadows {
        commands
            .entity(*shadow)
            .insert(RenderLayers::layer(HIDDEN_LAYER));
    }
    commands.insert_resource(Capture {
        hash,
        target,
        cameras,
        source: CaptureSource::Scene {
            level: level.0,
            hidden,
            shadows,
        },
        frames: 0,
        tries: 0,
        done: false,
    });
}

#[derive(Resource)]
struct BuildingLevel(Task<Option<BuiltLevel>>);

struct BuiltLevel {
    id: u32,
    hash: u64,
    mesh: LevelMesh,
}

/// Starts to build the level of a map that `/track` has offered for a moment. (Not in an online
/// race, which a hitch would cost more than the minimap does.)
fn start_building(
    time: Res<Time<Real>>,
    loading: Option<Res<Loading>>,
    taking: Option<Res<Capture>>,
    building: Option<Res<BuildingLevel>>,
    lobby: Res<Lobby>,
    mut pictures: ResMut<TrackPictures>,
    mut commands: Commands,
) {
    if loading.is_some() || taking.is_some() || building.is_some() || lobby.online.is_some() {
        return;
    }
    let Some((id, since)) = pictures.awaiting_preview else {
        return;
    };
    let Some(hash) = maps::hash(id) else {
        return;
    };
    if time.elapsed_secs() - since < PREVIEW_DELAY || !pictures.previewed.insert(hash) {
        return;
    }
    commands.insert_resource(BuildingLevel(AsyncComputeTaskPool::get().spawn(
        async move {
            let map = maps::get(id)?;
            let mut vram = Vram::default();
            for image in &map.vram {
                vram.upload(image).ok()?;
            }
            upload_shared(&mut vram, &crate::disc::source());
            let mesh = mesh::level_mesh(&map.level, &map.dram, &vram, false, true, true);
            Some(BuiltLevel { id, hash, mesh })
        },
    )));
}

/// Where a preview looks from: behind and over the first place on the grid, along it.
fn preview_view(spawn: &loader::lev::SpawnPoint) -> Transform {
    let [x, y, z] = mesh::to_world(spawn.pos);
    // (A kart faces a quarter turn from its spawn point: `vehicle::sim::spawn`.)
    let yaw = ((spawn.rot.y as i32 + 0x400) & 0xfff) as f32 / 4096.0 * std::f32::consts::TAU;
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let at = Vec3::new(x, y, z);
    Transform::from_translation(at - forward * PREVIEW_BEHIND + Vec3::Y * PREVIEW_ABOVE).looking_at(
        at + forward * PREVIEW_AHEAD + Vec3::Y * PREVIEW_LOOK_ABOVE,
        Vec3::Y,
    )
}

/// Spawns the level that was built, and the cameras of its picture.
fn spawn_preview(
    mut building: ResMut<BuildingLevel>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut subtract_materials: ResMut<Assets<SubtractMaterial>>,
    mut commands: Commands,
) {
    let Some(built) = check_ready(&mut building.0) else {
        return;
    };
    commands.remove_resource::<BuildingLevel>();
    let Some(BuiltLevel { id, hash, mesh }) = built else {
        return;
    };
    let Some(map) = maps::get(id) else {
        return;
    };
    let atlas = atlas_image(mesh.atlas, &mut images);
    let base = StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas),
        unlit: true,
        cull_mode: Some(bevy::render::render_resource::Face::Back),
        double_sided: false,
        ..default()
    };
    let root = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    spawn_layers(
        mesh.parts,
        &mesh.biases,
        &base,
        Transform::default(),
        Some(root),
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
        PREVIEW_LAYER,
    );
    spawn_parts(
        mesh.water,
        &base,
        Transform::default(),
        Some(root),
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut subtract_materials,
        PREVIEW_LAYER,
    );
    let target = capture_target(&mut images);
    let view = preview_view(&map.level.driver_spawns[0]);
    // As in `scene::setup`.
    let mut projection = PerspectiveProjection {
        fov: 2.0 * (0.75f32).atan(),
        ..default()
    };
    let dome = map.level.skybox.as_ref().and_then(|skybox| {
        sky::spawn_dome(
            &mut commands,
            &mut meshes,
            &mut materials,
            skybox,
            PREVIEW_SKY_LAYER,
            Some(root),
        )
    });
    let [r, g, b] = map.level.clear_colors[0].unwrap_or_default();
    let clear = ClearColorConfig::Custom(Color::srgb_u8(r, g, b));
    let mut cameras = vec![
        commands
            .spawn((
                camera(
                    -10,
                    if dome.is_some() {
                        ClearColorConfig::None
                    } else {
                        clear.clone()
                    },
                    &target,
                ),
                Projection::Perspective(projection.clone()),
                view,
                RenderLayers::layer(PREVIEW_LAYER),
            ))
            .id(),
    ];
    if let Some(radius) = dome {
        projection.far = projection.far.max(radius * 2.0);
        cameras.push(
            commands
                .spawn((
                    camera(-11, clear, &target),
                    Projection::Perspective(projection),
                    Transform::from_rotation(view.rotation),
                    RenderLayers::layer(PREVIEW_SKY_LAYER),
                ))
                .id(),
        );
    }
    commands.insert_resource(Capture {
        hash,
        target,
        cameras,
        source: CaptureSource::Preview(root),
        frames: 0,
        tries: 0,
        done: false,
    });
}

/// Takes the picture, and puts everything back.
fn finish_capture(
    mut taking: ResMut<Capture>,
    level: Res<LevelNumber>,
    mut karts: Query<&mut Visibility>,
    mut commands: Commands,
) {
    taking.frames += 1;
    // (Another level's scene took everything of this one with it, a preview too.)
    let cut_short = match &taking.source {
        CaptureSource::Scene { level: of, .. } => *of != level.0,
        CaptureSource::Preview(root) => commands.get_entity(*root).is_err(),
    };
    let preview = matches!(taking.source, CaptureSource::Preview(_));
    if !cut_short && taking.frames == taking.settle_frames() {
        let hash = taking.hash;
        commands
            .spawn(Screenshot::image(taking.target.clone()))
            .observe(
                move |captured: On<ScreenshotCaptured>,
                      mut taking: Option<ResMut<Capture>>,
                      mut pictures: ResMut<TrackPictures>,
                      mut images: ResMut<Assets<Image>>| {
                    let image = &captured.image;
                    // A scene's is not waited for; a preview's that was cut short is not kept.
                    let Some(taking) = taking.as_deref_mut().filter(|t| preview && t.hash == hash)
                    else {
                        if !preview {
                            keep_capture(hash, false, image, &mut pictures, &mut images);
                        }
                        return;
                    };
                    if lit_fraction(image) >= PREVIEW_MIN_LIT {
                        keep_capture(hash, true, image, &mut pictures, &mut images);
                        taking.done = true;
                    } else if taking.tries + 1 < PREVIEW_TRIES {
                        taking.tries += 1;
                        taking.frames = 0;
                    } else {
                        taking.done = true;
                    }
                },
            );
    }
    let waiting = if preview {
        !taking.done && taking.frames < PREVIEW_GIVE_UP_FRAMES
    } else {
        taking.frames <= taking.settle_frames()
    };
    if !cut_short && waiting {
        return;
    }
    match &taking.source {
        CaptureSource::Scene {
            hidden, shadows, ..
        } => {
            for (kart, was) in hidden {
                if let Ok(mut visibility) = karts.get_mut(*kart) {
                    *visibility = *was;
                }
            }
            for shadow in shadows {
                if let Ok(mut shadow) = commands.get_entity(*shadow) {
                    shadow.remove::<RenderLayers>();
                }
            }
        }
        CaptureSource::Preview(root) => {
            if let Ok(mut root) = commands.get_entity(*root) {
                root.try_despawn();
            }
        }
    }
    for camera in &taking.cameras {
        commands.entity(*camera).try_despawn();
    }
    commands.remove_resource::<Capture>();
}

fn lit_fraction(image: &Image) -> f32 {
    let Some(data) = &image.data else {
        return 0.0;
    };
    let lit = data.chunks(4).filter(|p| p[..3] != [0; 3]).count();
    lit as f32 / (data.len() / 4).max(1) as f32
}

fn keep_capture(
    hash: u64,
    preview: bool,
    image: &Image,
    pictures: &mut TrackPictures,
    images: &mut Assets<Image>,
) {
    let Some(data) = &image.data else {
        return;
    };
    let mut rgba = data.clone();
    for texel in rgba.chunks_mut(4) {
        texel[3] = 255;
    }
    let size = image.size();
    store::save(hash, &encode(hash, size, preview, &rgba));
    let picture = captured_picture(size, rgba, images);
    pictures
        .captured
        .insert(hash, Some(CapturedPicture { picture, preview }));
}

pub struct TrackPicturePlugin;

impl Plugin for TrackPicturePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrackPictures>().add_systems(
            Update,
            (
                start_capture,
                start_building,
                spawn_preview.run_if(resource_exists::<BuildingLevel>),
                finish_capture.run_if(resource_exists::<Capture>),
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_picture_reads_back() {
        let rgba = vec![7; 2 * 3 * 4];
        let bytes = encode(0x1234, UVec2::new(2, 3), true, &rgba);
        assert_eq!(u64::from_le_bytes(bytes[..8].try_into().unwrap()), 0x1234);
        assert_eq!(decode(&bytes), Some((UVec2::new(2, 3), true, rgba)));
        // Too short for its size, or not one at all.
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None);
        assert_eq!(decode(&[]), None);
    }
}
