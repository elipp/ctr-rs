//! Loading the disc: the levels, the racers and the pictures they share.

use std::path::PathBuf;

use crate::{
    FlameModels, Racer, ShadowIcons, SmokeIcon, TiremarkIcon, WheelIcons, flame, font, maps, skid,
};
use bevy::prelude::*;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::icons::{Icon, parse_groups, parse_icons};
use loader::lev::Level;
use loader::level::LevelId;
use loader::model::Model;
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

/// What a racer is made of: everything that comes from the character's own files. It can be
/// loaded again without the level.
pub(crate) struct Character {
    pub(crate) racer: Model,
    /// The icons of the racer's wheel sprites (`tireAnim`); none for Oxide, who has no wheels.
    pub(crate) wheel_icons: Vec<Icon>,
    /// Global icons 0 and 1 of the racer's pack (`shadow1`, `shadow2`).
    pub(crate) shadow_icons: Option<[Icon; 2]>,
    /// The first picture of the `dustpuff` icon group: the exhaust smoke.
    pub(crate) smoke_icon: Option<Icon>,
    /// Global icon `tiremark` of the racer's pack: the skid marks.
    pub(crate) tiremark_icon: Option<Icon>,
    /// `turbo` .. `turbo7` of the racer's pack: the frames of the exhaust fire.
    pub(crate) flame_models: Vec<Model>,
}

impl Character {
    /// Makes this the racer on the grid: the resources the kart is spawned from.
    pub(crate) fn insert(self, world: &mut World) {
        world.insert_resource(Racer(self.racer));
        world.insert_resource(WheelIcons(self.wheel_icons));
        world.insert_resource(ShadowIcons(self.shadow_icons));
        world.insert_resource(SmokeIcon(self.smoke_icon));
        world.insert_resource(TiremarkIcon(self.tiremark_icon));
        world.insert_resource(FlameModels(self.flame_models));
    }
}

/// What every racer's pack holds the same of, loaded once: neither a level nor a character
/// changes it.
pub(crate) struct Shared {
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
    /// The menus' text.
    pub(crate) fonts: font::Fonts,
    /// The big digits of the place in a race.
    pub(crate) digits: Vec<render::place::Digit>,
}

pub(crate) fn bigfile_path() -> PathBuf {
    crate::disc::path()
}

/// The model and the wheel sprites of a racer (none for Oxide, who has no wheels), for the
/// karts of the other players.
pub(crate) fn load_racer(character: usize) -> (Model, Vec<Icon>) {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file = big
        .read(
            &mut disc,
            BigIndex::RACER_MODEL_HI.nth(character as u32).get(),
        )
        .expect("read racer model");
    let model =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let wheel_icons = if character as u32 == NITROS_OXIDE {
        Vec::new()
    } else {
        let pack = big
            .read(
                &mut disc,
                BigIndex::ARCADE_1P_PACK.nth(character as u32).get(),
            )
            .expect("read racer pack");
        parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons")
            .into_iter()
            .find(|g| g.group_id == 0)
            .expect("racer pack has the tire icon group")
            .icons
    };
    (model, wheel_icons)
}

/// A level from the disc, with the texture page file shared by every track (`shared.vrm`): the
/// racers' textures are in it.
pub(crate) fn load_level(id: u32) -> LevelData {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    // (A map that an online race is on and we lack is fetched first, see `lobby.rs`.)
    assert!(
        !map::is_map_id(id) || maps::get(id).is_some(),
        "map {id:#x} is not in the maps folder"
    );
    if let Some(map) = maps::get(id) {
        debug!("level {id:#x} ({}): loaded from map cache", maps::name(id));
        // A map brings its own level and texture pages; the shared ones are the disc's.
        let mut vram = Vram::default();
        for img in &map.vram {
            vram.upload(img).expect("upload VRAM image");
        }
        upload_shared(&mut vram, &big, &mut disc);
        return LevelData {
            level: map.level.clone(),
            dram: map.dram.clone(),
            vram,
        };
    }
    debug!("level {id:#x} ({}): loaded from disc", maps::name(id));
    let mut read = |f| {
        let index = BigIndex::level(LevelId(id), Lod::OnePlayer, f);
        big.read(&mut disc, index.get())
            .expect("read level file from BIGFILE")
    };
    let dram = DramFile::parse(&read(LevFile::Lev)).expect("parse DRAM file");
    let level = Level::parse(&dram).expect("parse LEV");
    let mut vram = Vram::default();
    for img in parse_vram_file(&read(LevFile::Vram)).expect("parse VRAM file") {
        vram.upload(&img).expect("upload VRAM image");
    }
    upload_shared(&mut vram, &big, &mut disc);
    LevelData { level, dram, vram }
}

/// The texture pages that every track shares (`shared.vrm`).
pub(crate) fn upload_shared(
    vram: &mut Vram,
    big: &loader::bigfile::BigFile,
    disc: &mut loader::disc::Disc,
) {
    let shared = big
        .read(disc, BigIndex::SHARED_MPK_VRM.get())
        .expect("read shared VRAM file");
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
}

/// A racer from the disc: its model, and what its pack has of the kart's.
pub(crate) fn load_character(character: u32) -> Character {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let file = big
        .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
        .expect("read racer model");
    let racer =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(character).get())
        .expect("read racer pack");
    let pack = DramFile::parse(&pack).expect("parse racer pack");
    let icons = parse_icons(&pack).expect("parse racer pack icons");
    let groups = parse_groups(&pack).expect("parse racer pack icons");
    let global = |index| icons.iter().find(|i| i.global_index == index).cloned();
    let group = |id| groups.iter().find(|g| g.group_id == id);
    Character {
        wheel_icons: if character == NITROS_OXIDE {
            Vec::new()
        } else {
            group(0)
                .expect("racer pack has the tire icon group")
                .icons
                .clone()
        },
        shadow_icons: global(0).zip(global(1)).map(|(a, b)| [a, b]),
        smoke_icon: group(1).and_then(|g| g.icons.first().cloned()),
        tiremark_icon: global(skid::TIREMARK_ICON),
        flame_models: flame::models(&pack).expect("racer pack has the turbo models"),
        racer,
    }
}

/// What the racers' packs share. (Character 0's pack will do.)
pub(crate) fn load_shared() -> Shared {
    let path = bigfile_path();
    let (mut disc, big) = open_bigfile(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let pack = big
        .read(&mut disc, BigIndex::ARCADE_1P_PACK.nth(0).get())
        .expect("read racer pack");
    let pack = DramFile::parse(&pack).expect("parse racer pack");
    // The fonts are in the texture pages every level shares.
    let mut vram = Vram::default();
    let shared = big
        .read(&mut disc, BigIndex::SHARED_MPK_VRM.get())
        .expect("read shared.vrm");
    for image in parse_vram_file(&shared).expect("parse shared.vrm") {
        vram.upload(&image).expect("upload shared.vrm");
    }
    let groups = parse_groups(&pack).expect("parse racer pack icons");
    let font =
        |f| render::font::FontAtlas::new(f, &groups, &vram).expect("racer pack has the fonts");
    Shared {
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
    }
}
