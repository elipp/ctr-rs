//! The policy: an actor that gives each head of [`race::nn::HEADS`] its log-probabilities, and a
//! critic that values where the kart is. Two separate MLPs on the same observation.

use candle_core::{D, DType, Device, Result, Tensor};
use candle_nn::{Linear, Module, VarBuilder, VarMap, linear};
pub use race::nn::{HEAD_START, LOGITS};
use race::nn::{Action, HEADS, N_HEADS, OBS_LEN, Obs};

use crate::rng::Rng;

/// The boost's head (L1) among [`HEADS`].
const BOOST_HEAD: usize = 3;

/// A fresh policy's logit of pressing L1 (against 0 for not): about 10%. At an even chance
/// nearly every drift has its three tries tapped away before its meter is ready (each too
/// early a tap fails, and the meter starts over), and so never boosts; at this, most drifts
/// get to the meter's window and a tap by chance often lands in it.
const BOOST_START_LOGIT: f32 = -2.2;

/// Linear layers with tanh between them.
struct Mlp {
    layers: Vec<Linear>,
}

impl Mlp {
    fn new(vb: VarBuilder, sizes: &[usize]) -> Result<Mlp> {
        let layers = sizes
            .windows(2)
            .enumerate()
            .map(|(i, w)| linear(w[0], w[1], vb.pp(format!("l{i}"))))
            .collect::<Result<_>>()?;
        Ok(Mlp { layers })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let mut x = x.clone();
        for (i, layer) in self.layers.iter().enumerate() {
            x = layer.forward(&x)?;
            if i + 1 < self.layers.len() {
                x = x.tanh()?;
            }
        }
        Ok(x)
    }
}

pub struct Policy {
    actor: Mlp,
    critic: Mlp,
    pub device: Device,
}

/// What the actor chose for one observation.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub action: Action,
    /// The log-probability of all of it (the sum of the heads').
    pub log_prob: f32,
    pub value: f32,
}

impl Policy {
    /// A fresh policy in `vars`, `hidden` wide. The actor's last layer is scaled down, so that it
    /// starts out choosing about evenly, but for the boost: [`BOOST_START_LOGIT`].
    pub fn new(vars: &VarMap, hidden: usize, device: &Device) -> Result<Policy> {
        let vb = VarBuilder::from_varmap(vars, DType::F32, device);
        let actor = Mlp::new(vb.pp("actor"), &[OBS_LEN, hidden, hidden, LOGITS])?;
        let critic = Mlp::new(vb.pp("critic"), &[OBS_LEN, hidden, hidden, 1])?;
        let last = actor.layers.len() - 1;
        let (weight, bias) = (format!("actor.l{last}.weight"), format!("actor.l{last}.bias"));
        for (name, var) in vars.data().lock().unwrap().iter() {
            if *name == weight {
                var.set(&(var.as_tensor() * 0.01)?)?;
            } else if *name == bias {
                let mut b: Vec<f32> = var.as_tensor().to_vec1()?;
                b[HEAD_START[BOOST_HEAD]] = 0.0;
                b[HEAD_START[BOOST_HEAD] + 1] = BOOST_START_LOGIT;
                var.set(&Tensor::from_vec(b, LOGITS, device)?)?;
            }
        }
        Ok(Policy {
            actor,
            critic,
            device: device.clone(),
        })
    }

    /// The policy saved at `path` (`VarMap::save`, as `train` does), its width read off the
    /// weights: the actor's first layer is `[hidden, OBS_LEN]`.
    pub fn load(path: &std::path::Path, device: &Device) -> Result<Policy> {
        let saved = candle_core::safetensors::load(path, device)?;
        let first = saved
            .get("actor.l0.weight")
            .ok_or_else(|| candle_core::Error::Msg("no actor.l0.weight in the file".into()))?;
        let (hidden, obs) = first.dims2()?;
        if obs != OBS_LEN {
            return Err(candle_core::Error::Msg(format!(
                "the policy sees {obs} numbers, the race gives {OBS_LEN}: trained on another \
                 observation"
            )));
        }
        let mut vars = VarMap::new();
        let policy = Policy::new(&vars, hidden, device)?;
        // (The layers share the vars' storage, which `load` writes into: the map can go.)
        vars.load(path)?;
        Ok(policy)
    }

    /// The observations as a `[n, OBS_LEN]` tensor.
    pub fn batch(&self, obs: &[Obs]) -> Result<Tensor> {
        let flat: Vec<f32> = obs.iter().flatten().copied().collect();
        Tensor::from_vec(flat, (obs.len(), OBS_LEN), &self.device)
    }

    /// `[n, LOGITS]`.
    pub fn logits(&self, obs: &Tensor) -> Result<Tensor> {
        self.actor.forward(obs)
    }

    /// `[n]`.
    pub fn value(&self, obs: &Tensor) -> Result<Tensor> {
        self.critic.forward(obs)?.squeeze(D::Minus1)
    }

    /// Each head's log-probabilities, `[n, HEADS[h]]`.
    pub fn head_log_probs(logits: &Tensor) -> Result<Vec<Tensor>> {
        (0..N_HEADS)
            .map(|h| {
                candle_nn::ops::log_softmax(&logits.narrow(1, HEAD_START[h], HEADS[h])?, D::Minus1)
            })
            .collect()
    }

    /// A choice for each of `obs`: drawn from the heads' distributions, or with `greedy` the
    /// likeliest of each.
    pub fn act(&self, obs: &[Obs], greedy: bool, rng: &mut Rng) -> Result<Vec<Choice>> {
        let x = self.batch(obs)?;
        let heads = Self::head_log_probs(&self.logits(&x)?)?;
        let heads: Vec<Vec<Vec<f32>>> = heads.iter().map(|t| t.to_vec2()).collect::<Result<_>>()?;
        let values: Vec<f32> = self.value(&x)?.to_vec1()?;
        Ok((0..obs.len())
            .map(|i| {
                let mut choice = [0; N_HEADS];
                let mut log_prob = 0.0;
                for h in 0..N_HEADS {
                    let lp = &heads[h][i];
                    choice[h] = if greedy {
                        (0..lp.len()).max_by(|&a, &b| lp[a].total_cmp(&lp[b])).unwrap_or(0)
                    } else {
                        rng.categorical(lp)
                    };
                    log_prob += lp[choice[h]];
                }
                Choice {
                    action: Action::from_heads(choice),
                    log_prob,
                    value: values[i],
                }
            })
            .collect())
    }
}
