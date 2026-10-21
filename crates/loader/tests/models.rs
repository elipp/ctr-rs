mod common;

use loader::bigindex::BigIndex;
use loader::dram::DramFile;
use loader::model::Model;
use loader::vram::{Vram, parse_vram_file};

/// Every racer model parses at every detail level, its animation frames decode,
/// and everything its commands point at exists and is on a populated VRAM
/// page of the shared texture file.
#[test]
fn racer_models_parse_and_have_textures() {
    let Some(mut b) = common::open_bigfile() else { return };
    let mut vram = Vram::default();
    let shared = b.big.read(&mut b.disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
    for img in parse_vram_file(&shared).unwrap() {
        vram.upload(&img).unwrap();
    }

    for (lod, base) in [
        ("hi", BigIndex::RACER_MODEL_HI),
        ("med", BigIndex::RACER_MODEL_MED),
        ("low", BigIndex::RACER_MODEL_LOW),
    ] {
        for character in 0..16 {
            let what = format!("{lod} racer {character}");
            let file = b.big.read(&mut b.disc, base.nth(character).get()).unwrap();
            let model = Model::parse(&DramFile::parse(&file).unwrap()).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(!model.headers.is_empty(), "{what}");

            for header in &model.headers {
                let frames: Vec<_> = header
                    .static_frame
                    .iter()
                    .chain(header.animations.iter().flat_map(|a| &a.frames))
                    .collect();
                assert!(!frames.is_empty(), "{what}: no frames");
                for frame in frames {
                    let triangles = header.triangles(frame);
                    assert!(!triangles.is_empty(), "{what}");
                    for t in triangles {
                        assert!(t.texture <= header.textures.len(), "{what}: texture {}", t.texture);
                    }
                }
                // Texels of the textured faces resolve to something visible
                // somewhere (a missing page would read as all transparent).
                let visible = header.textures.iter().any(|t| {
                    (0..=255u8).step_by(5).any(|u| (0..=255u8).step_by(5).any(|v| matches!(vram.texel(t, u, v), Ok(Some(c)) if c[3] != 0)))
                });
                assert!(visible, "{what}: no texture has any visible texel in the shared VRAM");
            }
        }
    }
}

/// The racer pack carries the shared models (the turbo flame among them).
#[test]
fn packs_carry_the_turbo_flame_models() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let models = loader::model::parse_pack_models(&DramFile::parse(&file).unwrap()).unwrap();
        if character == 0 {
            for m in &models {
                eprintln!("{:?} id {} lods {}", m.name, m.id, m.headers.len());
            }
        }
        for id in 0x2c..=0x33 {
            assert!(models.iter().any(|m| m.id == id), "character {character}: no model {id:#x}");
        }
    }
}

/// Every racer's pack carries the model a crate breaks into (`STATIC_CRATE_EXPLOSION`, 0x26): a
/// 12-frame animation whose frames all have faces.
#[test]
fn every_racer_pack_has_the_crate_explosion() {
    let Some(mut b) = common::open_bigfile() else { return };
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let models = loader::model::parse_pack_models(&DramFile::parse(&file).unwrap()).unwrap();
        let model = models.iter().find(|m| m.id == 0x26).unwrap_or_else(|| panic!("racer {character}: no crate explosion"));
        assert_eq!(model.name, "crate_explosion");
        let header = &model.headers[0];
        assert_eq!(header.animations.len(), 1, "racer {character}");
        assert_eq!(header.animations[0].num_frames, 12, "racer {character}");
        for frame in &header.animations[0].frames {
            assert!(!header.triangles(frame).is_empty(), "racer {character}");
        }
    }
}

/// Every racer's pack has the Aku Aku and Uka Uka masks: a head that poses (the first frame of its
/// animation) and a beam that turns through 24 frames, textured from the shared VRAM.
#[test]
fn every_racer_pack_has_the_masks() {
    let Some(mut b) = common::open_bigfile() else { return };
    let mut vram = Vram::default();
    let shared = b.big.read(&mut b.disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
    for img in parse_vram_file(&shared).unwrap() {
        vram.upload(&img).unwrap();
    }
    for character in 0..16 {
        let file = b.big.read(&mut b.disc, BigIndex::ARCADE_1P_PACK.nth(character).get()).unwrap();
        let models = loader::model::parse_pack_models(&DramFile::parse(&file).unwrap()).unwrap();
        for (id, name, frames) in [(0x39, "akumouth", 13), (0x3a, "ukamouth", 13), (0x3e, "akubeam", 24), (0x40, "ukabeam", 24)] {
            let model = models.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("racer {character}: no model {id:#x}"));
            assert_eq!(model.name, name);
            let header = &model.headers[0];
            assert_eq!(header.animations[0].num_frames, frames, "{name}");
            assert!(!header.triangles(&header.animations[0].frames[0]).is_empty(), "{name}");
            let visible = header.textures.iter().any(|t| {
                (0..=255u8).step_by(5).any(|u| (0..=255u8).step_by(5).any(|v| matches!(vram.texel(t, u, v), Ok(Some(c)) if c[3] != 0)))
            });
            assert!(visible, "racer {character}: {name} has no visible texel in the shared VRAM");
        }
    }
}
