mod common;

use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use loader::texture::{FaceMode, face_polygon, resolve_texture};
use loader::vram::{Vram, parse_vram_file};

/// Every textured face of every race track resolves to a layout whose
/// corners decode through the level's own VRAM.
#[test]
fn track_textures_decode() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let read = |b: &mut common::Big, f| b.big.read(&mut b.disc, BigIndex::level(LevelId(id), Lod::OnePlayer, f).get()).unwrap();
        let mut vram = Vram::default();
        for img in parse_vram_file(&read(&mut b, LevFile::Vram)).unwrap() {
            vram.upload(&img).unwrap();
        }
        let dram = DramFile::parse(&read(&mut b, LevFile::Lev)).unwrap();
        let lev = Level::parse(&dram).unwrap();

        let (mut textured, mut opaque) = (0, 0);
        for q in &lev.quad_blocks {
            for face in 0..4 {
                let ctx = |e: &dyn std::fmt::Display| format!("level {id} block {} face {face}: {e}", q.block_id);
                let Some(t) = resolve_texture(&dram.data, q.texture_mid[face]).unwrap_or_else(|e| panic!("{}", ctx(&e))) else {
                    continue;
                };
                face_polygon(face, FaceMode::of(q, face)).unwrap_or_else(|e| panic!("{}", ctx(&e)));
                for (u, v) in t.uv {
                    textured += 1;
                    opaque += vram.texel(&t, u, v).unwrap_or_else(|e| panic!("{}", ctx(&e))).is_some() as usize;
                }
            }
        }
        assert!(textured > 0, "level {id} has no textures");
        assert!(opaque * 2 > textured, "level {id}: only {opaque}/{textured} texels are opaque");
    }
}
