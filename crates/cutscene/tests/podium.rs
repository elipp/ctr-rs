//! The podium's scripts of the disc, run on the disc's dance models. Skipped without the disc.

#[path = "../../loader/tests/common/mod.rs"]
mod common;

use cutscene::{DANCERS, Dancer, Event, Frames, Scripts};
use loader::bigindex::BigIndex;
use loader::dram::DramFile;
use loader::model::Model;
use render::particles::Entry;

struct Of<'a>(&'a Model);

impl Frames for Of<'_> {
    fn num_frames(&self, lod: usize, anim: usize) -> i32 {
        self.0
            .headers
            .get(lod)
            .and_then(|h| h.animations.get(anim))
            .map_or(0, |a| a.num_frames as i32)
    }
    fn lods(&self) -> usize {
        self.0.headers.len()
    }
}

/// A minute of each dance, on each step, and of the trophy girl: every opcode they reach is one
/// the interpreter knows, and the poses are in the models.
#[test]
fn needs_disc_every_podium_script_runs_on_its_model() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let mut read = |index: BigIndex| b.big.read(&mut b.disc, index.get()).unwrap();
    let scripts = Scripts::parse(read(BigIndex::OVERLAY_SECT3.nth(3))).unwrap();
    let model = |raw: Vec<u8>| Model::parse(&DramFile::parse(&raw).unwrap()).unwrap();
    let mut runs = vec![(
        "tawna".to_string(),
        scripts.tawna(),
        model(read(BigIndex::DANCE_TAWNA_GIRL.nth(1))),
    )];
    for character in 0..DANCERS as u32 {
        // (Oxide has no dance for the top step.)
        if character != 15 {
            let raw = read(BigIndex::DANCE_MODEL_WIN.nth(character * 2 + 1));
            let script = scripts.dance(character as usize, true).unwrap();
            runs.push((format!("first {character}"), script, model(raw)));
        }
        let raw = read(BigIndex::DANCE_MODEL_LOSE.nth(character * 2 + 1));
        let script = scripts.dance(character as usize, false).unwrap();
        runs.push((format!("other {character}"), script, model(raw)));
    }
    for (what, script, model) in runs {
        let mut rng = 100;
        let mut dancer = Dancer::new(&scripts, script, 0, &mut rng).unwrap();
        let mut events = Vec::new();
        let mut frames = std::collections::HashSet::new();
        for tick in 0..30 * 60 {
            dancer
                .tick(&scripts, &Of(&model), 32, &mut rng, &mut events)
                .unwrap_or_else(|e| panic!("{what}, tick {tick}: {e}"));
            let header = &model.headers[dancer.lod()];
            let anim = header.animations.get(dancer.anim);
            assert!(
                anim.is_some_and(|a| (dancer.frame as u16) < a.num_frames),
                "{what}, tick {tick}: animation {} frame {}",
                dancer.anim,
                dancer.frame
            );
            frames.insert((dancer.anim, dancer.frame));
        }
        assert!(frames.len() > 10, "{what} hardly moves: {frames:?}");
        assert!(
            events
                .iter()
                .all(|e| matches!(e, Event::Play(id) | Event::Stop(id) if *id < 0x200)),
            "{what}: {events:?}"
        );
    }
}

/// The particles of the podium's scripts: those of every id a script sets read as the emitters
/// that `render::particles` makes.
#[test]
fn needs_disc_the_podium_particles_read_as_emitters() {
    let Some(mut b) = common::open_bigfile() else {
        return;
    };
    let scripts = Scripts::parse(
        b.big
            .read(&mut b.disc, BigIndex::OVERLAY_SECT3.nth(3).get())
            .unwrap(),
    )
    .unwrap();
    // N. Gin's dances (0 and 2), Joe's on a lower step (3, which goes on to 4), N. Tropy's on top (5).
    for id in [0, 2, 3, 5] {
        let bursts = scripts.particles(id).unwrap();
        assert!(!bursts.is_empty(), "{id}");
        for burst in bursts {
            let set = Entry::parse_set(scripts.overlay(), burst.emitters)
                .unwrap_or_else(|e| panic!("{id}: {e}"));
            assert!(set.iter().any(|e| matches!(e, Entry::Func { .. })), "{id}");
        }
    }
    assert_eq!(scripts.particles(0).unwrap().len(), 2);
    assert!(scripts.particles(-1).unwrap().is_empty());
}
