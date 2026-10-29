use loader::lev::Level;
use loader::vram::Vram;

use super::atlas::TILE;
use super::*;
use loader::bigindex::{BigIndex, LevFile, Lod};
use loader::disc::open_bigfile;
use loader::dram::DramFile;
use loader::level::LevelId;
use loader::vram::parse_vram_file;

#[test]
fn racer_models_mesh_with_valid_atlas() {
    use loader::model::Model;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
    let Ok((mut disc, big)) = open_bigfile(&path) else {
        return;
    };
    let mut vram = Vram::default();
    let shared = big.read(&mut disc, BigIndex::SHARED_MPK_VRM.get()).unwrap();
    for img in parse_vram_file(&shared).unwrap() {
        vram.upload(&img).unwrap();
    }
    for character in 0..16 {
        let file = big
            .read(&mut disc, BigIndex::RACER_MODEL_HI.nth(character).get())
            .unwrap();
        let model = Model::parse(&DramFile::parse(&file).unwrap()).unwrap();
        let header = &model.headers[0];
        let frame = &header.animations[0].frames[10];
        let m = model_mesh(header, frame, &vram, [0xccc; 3]);
        let positions = model_positions(header, frame, [0xccc; 3]);
        for (part, p) in m.parts.iter().zip(&positions) {
            assert_eq!(&part.positions, p, "racer {character}");
        }
        // Another pose has the same faces in the same order.
        let other = model_positions(header, &header.animations[0].frames[3], [0xccc; 3]);
        for (a, b) in positions.iter().zip(&other) {
            assert_eq!(a.len(), b.len(), "racer {character}");
        }
        let vertices: usize = m.parts.iter().map(|p| p.positions.len()).sum();
        assert!(
            vertices >= 3 * header.triangles(frame).len(),
            "racer {character}"
        );
        for p in &m.parts {
            assert_eq!(p.positions.len(), p.uvs.len());
            assert_eq!(p.positions.len(), p.colors.len());
            assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
            assert!(p.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
        }
        // A kart is a few world units across (the track is hundreds).
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in m.parts.iter().flat_map(|p| &p.positions) {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let size = [0, 1, 2].map(|a| hi[a] - lo[a]);
        println!(
            "racer {character}: size {size:?}, atlas {}x{}",
            m.atlas.width, m.atlas.height
        );
        assert!(
            size.iter().all(|&s| (0.3..3.0).contains(&s)),
            "racer {character}: {size:?}"
        );
    }
}

#[test]
fn real_tracks_mesh_with_valid_atlas() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
    let Ok((mut disc, big)) = open_bigfile(&path) else {
        return;
    };
    for id in 0..=17 {
        let mut read = |f| {
            big.read(
                &mut disc,
                BigIndex::level(LevelId(id), Lod::OnePlayer, f).get(),
            )
            .unwrap()
        };
        let dram = DramFile::parse(&read(LevFile::Lev)).unwrap();
        let level = Level::parse(&dram).unwrap();
        let mut vram = Vram::default();
        for img in parse_vram_file(&read(LevFile::Vram)).unwrap() {
            vram.upload(&img).unwrap();
        }
        let m = level_mesh(&level, &dram, &vram, false, true, true);
        for p in m.parts.iter().chain(&m.water) {
            assert_eq!(p.positions.len(), p.uvs.len());
            assert_eq!(p.positions.len(), p.colors.len());
            assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len()));
            assert!(p.uvs.iter().flatten().all(|c| (0.0..=1.0).contains(c)));
        }
        for corner in m.water_corners.values().flatten() {
            assert!(corner.slot < m.water[corner.part].positions.len());
        }
        let has_water = level.water.env_map.is_some();
        assert_eq!(
            has_water,
            m.water.iter().any(|p| !p.indices.is_empty()),
            "level {id}"
        );
        assert_eq!(m.parts.len(), m.biases.len() * PARTS);
        for corner in m.scenery_corners.values().flatten() {
            assert!(
                corner.slot < m.parts[corner.part].positions.len(),
                "level {id}"
            );
        }
        for f in &m.animated_faces {
            assert!(f.first_vertex + 4 <= m.parts[f.part].positions.len());
            assert_eq!(
                m.animations[f.animation].uvs.len(),
                m.animations[f.animation].texture.frames.len()
            );
        }
        assert!(
            m.animations
                .iter()
                .flat_map(|a| a.uvs.iter().flatten().flatten())
                .all(|c| (0.0..=1.0).contains(c))
        );
        assert!(
            m.atlas.height <= 8192,
            "level {id}: atlas {}x{}",
            m.atlas.width,
            m.atlas.height
        );
        assert!(
            m.atlas.height > TILE,
            "level {id}: no textures were decoded"
        );
        // The faces face one way: of the triangles that lie flat, most point up.
        let (mut up, mut down) = (0, 0);
        for part in &m.parts {
            for t in part.indices.chunks(3) {
                let [a, b, c] = [0, 1, 2].map(|i| part.positions[t[i] as usize]);
                let (u, v) = (
                    std::array::from_fn::<f32, 3, _>(|k| b[k] - a[k]),
                    std::array::from_fn::<f32, 3, _>(|k| c[k] - a[k]),
                );
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if n[1].abs() > 0.9 * length {
                    if n[1] > 0.0 { up += 1 } else { down += 1 }
                }
            }
        }
        assert!(
            up > down,
            "level {id}: flat triangles facing up {up}, down {down}"
        );
        println!(
            "level {id}: verts {:?}, biases {:?}, {} animations on {} faces, atlas {}x{}",
            m.parts
                .iter()
                .map(|p| p.positions.len())
                .collect::<Vec<_>>(),
            m.biases,
            m.animations.len(),
            m.animated_faces.len(),
            m.atlas.width,
            m.atlas.height
        );
    }
}
