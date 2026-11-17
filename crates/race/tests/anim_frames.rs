//! `race::track::ANIM_NUM_FRAMES` is what the disc's kart models say. Skipped without the disc
//! image.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use loader::bigindex::BigIndex;
use loader::dram::DramFile;
use loader::model::Model;
use race::track::{ANIM_NUM_FRAMES, anim_num_frames};

#[test]
fn needs_disc_anim_num_frames_are_the_models() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    for (character, expected) in (0..).zip(ANIM_NUM_FRAMES) {
        let file = b.big.read(&mut b.disc, BigIndex::RACER_MODEL_HI.nth(character).get()).unwrap();
        let model = Model::parse(&DramFile::parse(&file).unwrap()).unwrap();
        assert_eq!(anim_num_frames(&model), expected, "character {character}");
    }
}
