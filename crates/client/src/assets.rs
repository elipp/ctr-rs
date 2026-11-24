//! Loading the disc: the levels, the racers and the pictures they share.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{KartParts, flame, font, maps, skid};
use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::{parse_groups, parse_icons};
use loader::lev::Level;
use loader::model::Model;
use loader::source::{Need, Source};
use loader::vram::{Vram, parse_vram_file};

/// `NITROS_OXIDE` in `GAME_CHARACTER_IDS` order.
pub(crate) const NITROS_OXIDE: u32 = 15;

/// What a level is made of: its geometry, and the textures its pointers need (the level's own
/// and `shared.vrm`, which the racers' are in).
pub(crate) struct LevelData {
    pub(crate) level: Level,
    pub(crate) dram: DramFile,
    pub(crate) vram: Vram,
}

/// What a racer's kart has of its own; the rest is every racer's ([`KartParts`]). A custom racer
/// may come to have more of its own, or less.
pub(crate) struct Kit {
    pub(crate) character: usize,
    pub(crate) model: Model,
    /// Whether it has the wheel sprites: Oxide, who hovers, has none.
    pub(crate) wheels: bool,
}

impl Kit {
    /// The frames of each of its model's animations, which its kart's sim needs.
    pub(crate) fn anim_num_frames(&self) -> [u16; 8] {
        crate::scene::anim_num_frames(&self.model)
    }
}

/// The kits loaded so far, by character: a kart's every spawn takes its racer's from here.
#[derive(Resource, Default)]
pub(crate) struct Kits(HashMap<usize, Arc<Kit>>);

impl Kits {
    pub(crate) fn get(&mut self, character: usize) -> Arc<Kit> {
        self.0
            .entry(character)
            .or_insert_with(|| Arc::new(load_kit(character)))
            .clone()
    }
}

/// What every racer's pack holds the same of, loaded once: neither a level nor a character
/// changes it.
pub(crate) struct Shared {
    pub(crate) kart_parts: KartParts,
    /// `crate_explosion`: the break-up of a crate.
    pub(crate) crate_explosion: Option<Model>,
    /// The Aku Aku and Uka Uka masks.
    pub(crate) masks: render::mask::Masks,
    /// The shield bubble.
    pub(crate) shields: render::shield::Shields,
    /// The bombs, missiles and their bursts.
    pub(crate) projectiles: render::projectile::Models,
    /// The wumpa fruit of the counter, for a level that has none of its own.
    pub(crate) fruit_display: Model,
    /// The relic a relic race awards (`STATIC_RELIC`).
    pub(crate) relic: Model,
    /// The menus' text.
    pub(crate) fonts: font::Fonts,
    /// The big digits of the place in a race.
    pub(crate) digits: Vec<render::place::Digit>,
}

/// The bytes of a file of the disc.
fn get(source: &Source, need: Need) -> Vec<u8> {
    source
        .get(need)
        .unwrap_or_else(|e| panic!("{}: {e}", need.key()))
}

pub(crate) fn load_kit(character: usize) -> Kit {
    let file = get(&crate::disc::source(), Need::Racer(character as u32));
    let model =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    Kit {
        character,
        model,
        wheels: character as u32 != NITROS_OXIDE,
    }
}

/// A level, from its map, with the texture page file shared by every track (`shared.vrm`): the
/// racers' textures are in it. Its instances are as `mode` leaves them: a relic race and a time
/// trial are on the map's relic LEV where it has one (`Map::relic`), as retail's `levelLOD` 8 is
/// for both (`LOAD_TenStages`).
pub(crate) fn load_level(id: u32, mode: instance::lev_init::LevMode) -> LevelData {
    // (A map that an online race is on and we lack is fetched first, see `lobby.rs`.)
    let map = maps::get(id).unwrap_or_else(|| {
        if map::is_map_id(id) {
            panic!("map {id:#x} is not in the tracks folder")
        } else {
            panic!(
                "the map of the game's track {id} is missing: it is made from the disc when the \
                 client starts (delete the folder of the maps made from the disc to make them again)"
            )
        }
    });
    debug!("level {id:#x} ({}): loaded from its track file", maps::name(id));
    let (level, dram, images) = sim_level(map, id, mode);
    // A map brings its own level and texture pages; the shared ones are the disc's.
    let mut vram = Vram::default();
    for img in images {
        vram.upload(img).expect("upload VRAM image");
    }
    upload_shared(&mut vram, &crate::disc::source());
    LevelData {
        level,
        dram: dram.clone(),
        vram,
    }
}

/// The level a race of `mode` on map `id` runs on, and its texture pages
/// (`race::track::sim_level`).
pub(crate) fn sim_level(
    map: &map::Map,
    id: u32,
    mode: instance::lev_init::LevMode,
) -> (Level, &DramFile, &[loader::vram::VramImage]) {
    let made = race::track::sim_level(map, id, mode);
    for why in made.left_out {
        warn!("level {id:#x}: a bot path amendment is left out: {why}");
    }
    (made.level, made.dram, made.vram)
}

/// The texture pages that every track shares (`shared.vrm`).
pub(crate) fn upload_shared(vram: &mut Vram, source: &Source) {
    let shared = get(source, Need::SharedVram);
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
}

fn kart_parts(pack: &DramFile) -> KartParts {
    let icons = parse_icons(pack).expect("parse racer pack icons");
    let groups = parse_groups(pack).expect("parse racer pack icons");
    let global = |index| icons.iter().find(|i| i.global_index == index).cloned();
    let group = |id| groups.iter().find(|g| g.group_id == id);
    KartParts {
        wheels: group(0)
            .expect("racer pack has the tire icon group")
            .icons
            .clone(),
        shadow: global(0).zip(global(1)).map(|(a, b)| [a, b]),
        smoke: group(1).and_then(|g| g.icons.first().cloned()),
        tiremark: global(skid::TIREMARK_ICON),
        flames: flame::models(pack).expect("racer pack has the turbo models"),
    }
}

/// What the racers' packs share. (Character 0's pack will do.)
pub(crate) fn load_shared() -> Shared {
    let source = crate::disc::source();
    let pack = DramFile::parse(&get(&source, Need::Pack(0))).expect("parse racer pack");
    // The fonts are in the texture pages every level shares.
    let mut vram = Vram::default();
    upload_shared(&mut vram, &source);
    let groups = parse_groups(&pack).expect("parse racer pack icons");
    let font =
        |f| render::font::FontAtlas::new(f, &groups, &vram).expect("racer pack has the fonts");
    Shared {
        kart_parts: kart_parts(&pack),
        fonts: font::Fonts::new(
            font(render::font::Font::Large),
            font(render::font::Font::Middle),
        ),
        digits: render::place::digits(&pack).expect("racer pack has the big digits"),
        crate_explosion: render::explosion::model(&pack)
            .expect("racer pack has the crate explosion"),
        masks: render::mask::models(&pack).expect("racer pack has the masks"),
        shields: render::shield::models(&pack).expect("racer pack has the shields"),
        projectiles: render::projectile::models(&pack).expect("racer pack has the weapons"),
        fruit_display: render::fruit::display_model(&pack).expect("racer pack has the fruit"),
        relic: loader::model::parse_pack_models(&pack)
            .expect("parse racer pack models")
            .into_iter()
            .find(|m| m.id == crate::solo_end::RELIC_MODEL)
            .expect("racer pack has the relic"),
    }
}
