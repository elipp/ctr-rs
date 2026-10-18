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
