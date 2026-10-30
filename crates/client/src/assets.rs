//! Loading the disc: the levels, the racers and the pictures they share.

use crate::{
    FlameModels, Racer, ShadowIcons, SmokeIcon, TiremarkIcon, WheelIcons, flame, font, maps, skid,
};
use bevy::prelude::*;
use loader::dram::DramFile;
use loader::icons::{Icon, parse_groups, parse_icons};
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

/// The bytes of a file of the disc.
fn get(source: &Source, need: Need) -> Vec<u8> {
    source
        .get(need)
        .unwrap_or_else(|e| panic!("{}: {e}", need.key()))
}

/// The model and the wheel sprites of a racer (none for Oxide, who has no wheels), for the
/// karts of the other players.
pub(crate) fn load_racer(character: usize) -> (Model, Vec<Icon>) {
    let source = crate::disc::source();
    let file = get(&source, Need::Racer(character as u32));
    let model =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let wheel_icons = if character as u32 == NITROS_OXIDE {
        Vec::new()
    } else {
        let pack = get(&source, Need::Pack(character as u32));
        parse_groups(&DramFile::parse(&pack).expect("parse racer pack"))
            .expect("parse racer pack icons")
            .into_iter()
            .find(|g| g.group_id == 0)
            .expect("racer pack has the tire icon group")
            .icons
    };
    (model, wheel_icons)
}

/// A level, from its map, with the texture page file shared by every track (`shared.vrm`): the
/// racers' textures are in it.
pub(crate) fn load_level(id: u32) -> LevelData {
    // (A map that an online race is on and we lack is fetched first, see `lobby.rs`.)
    let map = maps::get(id).unwrap_or_else(|| {
        if map::is_map_id(id) {
            panic!("map {id:#x} is not in the maps folder")
        } else {
            panic!(
                "the map of the game's track {id} is missing: it is made from the disc when the \
                 client starts (delete the folder of the maps made from the disc to make them again)"
            )
        }
    });
    debug!("level {id:#x} ({}): loaded from its map", maps::name(id));
    // A map brings its own level and texture pages; the shared ones are the disc's.
    let mut vram = Vram::default();
    for img in &map.vram {
        vram.upload(img).expect("upload VRAM image");
    }
    upload_shared(&mut vram, &crate::disc::source());
    LevelData {
        level: map.level.clone(),
        dram: map.dram.clone(),
        vram,
    }
}

/// The texture pages that every track shares (`shared.vrm`).
pub(crate) fn upload_shared(vram: &mut Vram, source: &Source) {
    let shared = get(source, Need::SharedVram);
    for img in parse_vram_file(&shared).expect("parse shared VRAM file") {
        vram.upload(&img).expect("upload shared VRAM image");
    }
}

/// A racer from the disc: its model, and what its pack has of the kart's.
pub(crate) fn load_character(character: u32) -> Character {
    let source = crate::disc::source();
    let file = get(&source, Need::Racer(character));
    let racer =
        Model::parse(&DramFile::parse(&file).expect("parse DRAM file")).expect("parse racer model");
    let pack = DramFile::parse(&get(&source, Need::Pack(character))).expect("parse racer pack");
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
    let source = crate::disc::source();
    let pack = DramFile::parse(&get(&source, Need::Pack(0))).expect("parse racer pack");
    // The fonts are in the texture pages every level shares.
    let mut vram = Vram::default();
    upload_shared(&mut vram, &source);
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
