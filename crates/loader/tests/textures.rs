mod common;

use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::dram::DramFile;
use loader::lev::Level;
use loader::level::LevelId;
use loader::texture::{FaceMode, face_polygon, resolve_low_texture, resolve_texture};
use loader::vram::{Vram, parse_vram_file};

/// Every textured face of every race track resolves to a layout whose
/// corners decode through the level's own VRAM.
#[test]
fn track_textures_decode() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let read = |b: &mut common::Big, f| {
            b.big
                .read(
                    &mut b.disc,
                    BigIndex::level(LevelId(id), Lod::OnePlayer, f).get(),
                )
                .unwrap()
        };
        let mut vram = Vram::default();
        for img in parse_vram_file(&read(&mut b, LevFile::Vram)).unwrap() {
            vram.upload(&img).unwrap();
        }
        let dram = DramFile::parse(&read(&mut b, LevFile::Lev)).unwrap();
        let lev = Level::parse(&dram).unwrap();

        let (mut textured, mut opaque) = (0, 0);
        for q in &lev.quad_blocks {
            let low = resolve_low_texture(&dram, q.texture_low)
                .unwrap_or_else(|e| panic!("level {id} block {}: {e}", q.block_id));
            if let Some(t) = low {
                for (u, v) in t.uv {
                    vram.texel(&t, u, v).unwrap_or_else(|e| {
                        panic!("level {id} block {} low texture: {e}", q.block_id)
                    });
                }
            }
            for face in 0..4 {
                let ctx = |e: &dyn std::fmt::Display| {
                    format!("level {id} block {} face {face}: {e}", q.block_id)
                };
                let Some(g) = resolve_texture(&dram, q.texture_mid[face])
                    .unwrap_or_else(|e| panic!("{}", ctx(&e)))
                else {
                    continue;
                };
                let t = *g.best();
                face_polygon(face, FaceMode::of(q, face)).unwrap_or_else(|e| panic!("{}", ctx(&e)));
                for (u, v) in t.uv {
                    textured += 1;
                    opaque += vram
                        .texel(&t, u, v)
                        .unwrap_or_else(|e| panic!("{}", ctx(&e)))
                        .is_some() as usize;
                }
            }
        }
        assert!(textured > 0, "level {id} has no textures");
        assert!(
            opaque * 2 > textured,
            "level {id}: only {opaque}/{textured} texels are opaque"
        );
    }
}

/// Water vertices of every track animate to colours inside the environment
/// map's texture region, and every frame of the table wraps cleanly.
#[test]
fn track_water_animates() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    for id in LevelId::DINGO_CANYON.0..=LevelId::TURBO_TRACK.0 {
        let idx = BigIndex::level(LevelId(id), Lod::OnePlayer, LevFile::Lev);
        let dram = DramFile::parse(&b.big.read(&mut b.disc, idx.get()).unwrap()).unwrap();
        let lev = Level::parse(&dram).unwrap();
        let Some(env) = lev.water.env_map else {
            assert!(lev.water.vertices.is_empty(), "level {id}");
            continue;
        };
        assert!(
            lev.water
                .vertices
                .iter()
                .all(|v| v.vertex < lev.vertices.len())
        );
        let (u0, v0) = env.uv[0];
        for tick in [0, 1, 7, 8, 100, 223, 224, 1000] {
            for c in lev.water.animate(tick, |_| true) {
                // The environment map is a 64x64 region; the animated UV may reach a little past it.
                let (du, dv) = (c.uv[0].wrapping_sub(u0), c.uv[1].wrapping_sub(v0));
                assert!(
                    du < 80 && dv < 80,
                    "level {id} tick {tick}: uv {:?} vs env ({u0},{v0})",
                    c.uv
                );
            }
        }
        // 28 frames of 8 ticks.
        assert_eq!(
            lev.water.animate(0, |_| true),
            lev.water.animate(224, |_| true),
            "level {id}"
        );
    }
}
