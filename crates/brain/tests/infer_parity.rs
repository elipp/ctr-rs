//! The hand-rolled actor the client drives with (`race::infer`) against the candle one it was
//! trained as (`brain::policy`): the same logits, to rounding, and the same greedy choices. A
//! change to the network on one side only shows here.

use std::path::{Path, PathBuf};

use brain::policy::Policy;
use brain::rng::Rng;
use candle_core::{Device, Tensor};
use candle_nn::VarMap;
use race::infer::{self, Actor};
use race::nn::{HEAD_START, HEADS, LOGITS, N_HEADS, Obs};

/// Observations to compare on.
const SAMPLES: usize = 500;

/// A logit may be off by this, plus as much relative to its size: the sums go in another order.
const TOLERANCE: f32 = 1e-5;

/// Two logits closer than this are a near-tie: the greedy choice between them may go either way.
const NEAR_TIE: f32 = 1e-4;

/// Observations about the size `nn::observe` gives (±1, now and then more).
fn observations(seed: u64) -> Vec<Obs> {
    let mut rng = Rng::new(seed);
    (0..SAMPLES)
        .map(|_| std::array::from_fn(|_| (rng.unit() * 2.0 - 1.0) * 1.5))
        .collect()
}

/// Checks the two on `path`'s policy.
fn compare(path: &Path) {
    let device = Device::Cpu;
    let policy = Policy::load(path, &device).unwrap();
    let actor = Actor::load(path).unwrap();
    let obs = observations(7);
    let candle: Vec<Vec<f32>> = policy
        .logits(&policy.batch(&obs).unwrap())
        .unwrap()
        .to_vec2()
        .unwrap();
    let greedy = policy.act(&obs, true, &mut Rng::new(0)).unwrap();

    let mut worst = 0.0f32;
    let mut near_ties = 0;
    for (i, o) in obs.iter().enumerate() {
        let ours = actor.logits(o);
        let theirs = &candle[i];
        assert_eq!(theirs.len(), LOGITS);
        for (j, (&a, &b)) in ours.iter().zip(theirs).enumerate() {
            let off = (a - b).abs();
            worst = worst.max(off);
            assert!(
                off <= TOLERANCE * (1.0 + b.abs()),
                "observation {i}, logit {j}: {a} here, {b} in candle"
            );
        }
        // The log-probabilities a drawn choice is made from.
        for h in 0..N_HEADS {
            let head = |l: &[f32]| l[HEAD_START[h]..][..HEADS[h]].to_vec();
            let (ours, theirs) = (infer::log_softmax(&head(&ours)), log_softmax(&head(theirs)));
            for (a, b) in ours.iter().zip(&theirs) {
                assert!((a - b).abs() <= 2.0 * TOLERANCE * (1.0 + b.abs()), "head {h}: {a}, {b}");
            }
        }

        let action = actor.act(o, true, &mut Rng::new(0));
        if action != greedy[i].action {
            let tied = (0..N_HEADS).any(|h| {
                let mut head = theirs[HEAD_START[h]..][..HEADS[h]].to_vec();
                head.sort_by(|a, b| b.total_cmp(a));
                head[0] - head[1] < NEAR_TIE
            });
            assert!(
                tied,
                "observation {i}: {action:?} here, {:?} in candle, and no near-tie",
                greedy[i].action
            );
            near_ties += 1;
        }
    }
    eprintln!(
        "{}: worst logit off by {worst:e}, {near_ties} of {SAMPLES} greedy choices differ at a \
         near-tie",
        path.display()
    );
    // (At most a stray one: more is a bug, whatever the ties.)
    assert!(near_ties <= SAMPLES / 100);
}

/// The log-probabilities as candle's `log_softmax` makes them, in f64 for a reference.
fn log_softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let sum: f64 = logits.iter().map(|&l| (l as f64 - max).exp()).sum();
    logits.iter().map(|&l| (l as f64 - max - sum.ln()) as f32).collect()
}

/// A fresh policy, saved as `train` saves one, but with every weight drawn at a trained one's
/// size: a fresh actor's last layer is scaled down so far (`Policy::new`) that all of its logits
/// are near 0, and its choices all near-ties.
#[test]
fn a_random_policy() {
    let device = Device::Cpu;
    let vars = VarMap::new();
    Policy::new(&vars, 256, &device).unwrap();
    for (name, var) in vars.data().lock().unwrap().iter() {
        let dims = var.as_tensor().dims().to_vec();
        let std = match dims[..] {
            [_, inputs] => 1.0 / (inputs as f64).sqrt(),
            _ => 0.1,
        };
        let drawn = Tensor::randn(0.0f32, std as f32, dims, &device).unwrap();
        var.set(&drawn).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let path = std::env::temp_dir().join(format!("infer-parity-{}.safetensors", std::process::id()));
    vars.save(&path).unwrap();
    compare(&path);
    let _ = std::fs::remove_file(&path);
}

/// A trained policy, if there is one: `BRAIN_POLICY`, or where `train` saves it. (Skipped, with
/// a word, if it is not there or candle cannot read it either: trained on another observation.)
#[test]
fn the_trained_policy() {
    let path = std::env::var("BRAIN_POLICY").map(PathBuf::from).unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/brain/policy.safetensors")
    });
    if !path.exists() {
        eprintln!("no trained policy at {}: skipped", path.display());
        return;
    }
    if let Err(e) = Policy::load(&path, &Device::Cpu) {
        eprintln!("{}: {e}: skipped", path.display());
        return;
    }
    compare(&path);
}

/// The actor refuses what is not a policy of this observation, rather than driving with it.
#[test]
fn a_wrong_file_is_refused() {
    assert!(Actor::from_safetensors(b"").is_err());
    assert!(Actor::from_safetensors(&[255; 16]).is_err());
}
