//! The actor of a learned policy (`brain::policy`), run without candle: what the host needs to
//! drive a learned bot, and no dependency. Training stays on candle in `brain`; its parity test
//! (`brain/tests/infer_parity.rs`) checks that the two agree.
//!
//! The actor is linear layers with tanh between them, [`OBS_LEN`] in and [`LOGITS`] out, saved
//! (`VarMap::save`) as safetensors: `actor.l{i}.weight` `[out, in]` row-major and
//! `actor.l{i}.bias` `[out]`, f32. The critic is in the file as well, and left there: driving
//! needs only the actor.
//!
//! The sums go in another order than candle's, so a choice between two near-equal logits may come
//! out the other way. That is fine: a learned bot's pad is made on the host alone (see
//! [`crate::nn`]), nothing has to come out the same on another machine.

use crate::nn::{Action, HEAD_START, HEADS, LOGITS, N_HEADS, OBS_LEN, Obs};
use crate::rng::Rng;

/// One linear layer: a row of `inputs` weights for each output, and a bias each.
struct Layer {
    inputs: usize,
    weight: Vec<f32>,
    bias: Vec<f32>,
}

impl Layer {
    /// `out = weight · x + bias`.
    fn forward(&self, x: &[f32], out: &mut Vec<f32>) {
        out.clear();
        out.extend(
            self.weight
                .chunks_exact(self.inputs)
                .zip(&self.bias)
                .map(|(row, b)| dot(row, x) + b),
        );
    }
}

/// With eight sums side by side, so that the compiler can put them in vector registers (it may
/// not reorder a single float sum).
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 8];
    let (a8, b8) = (a.chunks_exact(8), b.chunks_exact(8));
    let (a_rest, b_rest) = (a8.remainder(), b8.remainder());
    for (a, b) in a8.zip(b8) {
        for i in 0..8 {
            acc[i] += a[i] * b[i];
        }
    }
    let mut sum = ((acc[0] + acc[4]) + (acc[1] + acc[5])) + ((acc[2] + acc[6]) + (acc[3] + acc[7]));
    for (a, b) in a_rest.iter().zip(b_rest) {
        sum += a * b;
    }
    sum
}

/// The actor of a policy.
pub struct Actor {
    layers: Vec<Layer>,
}

impl Actor {
    /// The actor of the policy saved at `path`.
    pub fn load(path: &std::path::Path) -> Result<Actor, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        Actor::from_safetensors(&bytes)
    }

    /// The actor of a policy saved as safetensors: an 8-byte little-endian length, that much JSON
    /// (name → dtype, shape, the data's offsets), then the data.
    pub fn from_safetensors(bytes: &[u8]) -> Result<Actor, String> {
        let len = bytes
            .get(..8)
            .ok_or("not a safetensors file: too short")?;
        let len = u64::from_le_bytes(len.try_into().unwrap());
        let data_start = usize::try_from(len)
            .ok()
            .and_then(|len| len.checked_add(8))
            .filter(|&end| end <= bytes.len())
            .ok_or("not a safetensors file: its header runs past the end")?;
        let header = std::str::from_utf8(&bytes[8..data_start])
            .map_err(|_| "not a safetensors file: its header is not UTF-8")?;
        let header = json::parse(header).map_err(|e| format!("the safetensors header: {e}"))?;
        let data = &bytes[data_start..];

        let tensor = |name: &str, shape: &[usize]| -> Result<Vec<f32>, String> {
            let t = header.get(name).ok_or_else(|| format!("no {name} in the file"))?;
            let dtype = t.get("dtype").and_then(json::Value::as_str);
            if dtype != Some("F32") {
                return Err(format!("{name} is {dtype:?}, not F32"));
            }
            let got: Option<Vec<usize>> = t
                .get("shape")
                .and_then(json::Value::as_array)
                .and_then(|dims| dims.iter().map(json::Value::as_usize).collect());
            if got.as_deref() != Some(shape) {
                return Err(format!("{name} is {got:?}, not {shape:?}"));
            }
            let offsets: Option<Vec<usize>> = t
                .get("data_offsets")
                .and_then(json::Value::as_array)
                .and_then(|o| o.iter().map(json::Value::as_usize).collect());
            let raw = match offsets.as_deref() {
                Some(&[start, end])
                    if end.checked_sub(start) == Some(shape.iter().product::<usize>() * 4) =>
                {
                    data.get(start..end)
                }
                _ => None,
            }
            .ok_or_else(|| format!("{name}'s data offsets are wrong"))?;
            Ok(raw
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect())
        };
        // The shape of a layer's weight, read off the header: the bias and the next layer must
        // agree with it.
        let dims = |name: &str| -> Option<(usize, usize)> {
            let shape = header.get(name)?.get("shape")?.as_array()?;
            match shape {
                [out, inp] => Some((out.as_usize()?, inp.as_usize()?)),
                _ => None,
            }
        };

        let mut layers = Vec::new();
        let mut inputs = OBS_LEN;
        loop {
            let i = layers.len();
            let name = format!("actor.l{i}.weight");
            let Some((outputs, got)) = dims(&name) else {
                if header.get(&name).is_some() {
                    return Err(format!("{name} is not a matrix"));
                }
                break;
            };
            if got != inputs {
                return Err(if i == 0 {
                    format!(
                        "the policy sees {got} numbers, the race gives {OBS_LEN}: trained on \
                         another observation"
                    )
                } else {
                    format!("{name} takes {got} inputs, the layer before gives {inputs}")
                });
            }
            layers.push(Layer {
                inputs,
                weight: tensor(&name, &[outputs, inputs])?,
                bias: tensor(&format!("actor.l{i}.bias"), &[outputs])?,
            });
            inputs = outputs;
        }
        if layers.is_empty() {
            return Err("no actor.l0.weight in the file".into());
        }
        if inputs != LOGITS {
            return Err(format!(
                "the policy gives {inputs} logits, the race's heads take {LOGITS}"
            ));
        }
        Ok(Actor { layers })
    }

    /// All of the heads' logits for `obs` ([`HEAD_START`]).
    pub fn logits(&self, obs: &Obs) -> [f32; LOGITS] {
        let mut x = obs.to_vec();
        let mut y = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            debug_assert_eq!(x.len(), layer.inputs);
            layer.forward(&x, &mut y);
            if i + 1 < self.layers.len() {
                y.iter_mut().for_each(|v| *v = v.tanh());
            }
            std::mem::swap(&mut x, &mut y);
        }
        x.try_into().expect("checked in from_safetensors")
    }

    /// A choice for `obs`: the likeliest of each head with `greedy`, else drawn from the heads'
    /// distributions, as `brain::policy::Policy::act` does.
    pub fn act(&self, obs: &Obs, greedy: bool, rng: &mut Rng) -> Action {
        let logits = self.logits(obs);
        let mut choice = [0; N_HEADS];
        for h in 0..N_HEADS {
            let head = &logits[HEAD_START[h]..][..HEADS[h]];
            choice[h] = if greedy {
                // (The likeliest of the log-probabilities is the likeliest logit.)
                (0..head.len()).max_by(|&a, &b| head[a].total_cmp(&head[b])).unwrap_or(0)
            } else {
                rng.categorical(&log_softmax(head))
            };
        }
        Action::from_heads(choice)
    }
}

/// The log-probabilities of `logits`.
pub fn log_softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let log_sum = logits.iter().map(|l| (l - max).exp()).sum::<f32>().ln() + max;
    logits.iter().map(|l| l - log_sum).collect()
}

/// Just enough JSON for a safetensors header.
mod json {
    pub enum Value {
        Object(Vec<(String, Value)>),
        Array(Vec<Value>),
        String(String),
        Number(f64),
        /// `true`, `false`, `null`: nothing a header of ours needs.
        Other,
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }

        pub fn as_array(&self) -> Option<&[Value]> {
            match self {
                Value::Array(items) => Some(items),
                _ => None,
            }
        }

        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::String(s) => Some(s),
                _ => None,
            }
        }

        pub fn as_usize(&self) -> Option<usize> {
            match *self {
                Value::Number(n) if n >= 0.0 && n.fract() == 0.0 && n < usize::MAX as f64 => {
                    Some(n as usize)
                }
                _ => None,
            }
        }
    }

    pub fn parse(text: &str) -> Result<Value, String> {
        let mut p = Parser {
            s: text.as_bytes(),
            at: 0,
        };
        let value = p.value(0)?;
        p.space();
        if p.at != p.s.len() {
            return Err(format!("something after the end, at byte {}", p.at));
        }
        Ok(value)
    }

    /// Deeper than this is not a header.
    const MAX_DEPTH: usize = 32;

    struct Parser<'a> {
        s: &'a [u8],
        at: usize,
    }

    impl Parser<'_> {
        fn err<T>(&self, what: &str) -> Result<T, String> {
            Err(format!("{what} at byte {}", self.at))
        }

        fn space(&mut self) {
            while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.at) {
                self.at += 1;
            }
        }

        fn eat(&mut self, b: u8) -> bool {
            self.space();
            let ate = self.s.get(self.at) == Some(&b);
            self.at += ate as usize;
            ate
        }

        fn value(&mut self, depth: usize) -> Result<Value, String> {
            if depth > MAX_DEPTH {
                return self.err("nested too deep");
            }
            self.space();
            match self.s.get(self.at) {
                Some(b'{') => {
                    self.at += 1;
                    let mut fields = Vec::new();
                    if self.eat(b'}') {
                        return Ok(Value::Object(fields));
                    }
                    loop {
                        self.space();
                        let key = self.string()?;
                        if !self.eat(b':') {
                            return self.err("expected ':'");
                        }
                        fields.push((key, self.value(depth + 1)?));
                        if self.eat(b'}') {
                            return Ok(Value::Object(fields));
                        }
                        if !self.eat(b',') {
                            return self.err("expected ',' or '}'");
                        }
                    }
                }
                Some(b'[') => {
                    self.at += 1;
                    let mut items = Vec::new();
                    if self.eat(b']') {
                        return Ok(Value::Array(items));
                    }
                    loop {
                        items.push(self.value(depth + 1)?);
                        if self.eat(b']') {
                            return Ok(Value::Array(items));
                        }
                        if !self.eat(b',') {
                            return self.err("expected ',' or ']'");
                        }
                    }
                }
                Some(b'"') => Ok(Value::String(self.string()?)),
                Some(b'-' | b'0'..=b'9') => {
                    let start = self.at;
                    while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') =
                        self.s.get(self.at)
                    {
                        self.at += 1;
                    }
                    let text = std::str::from_utf8(&self.s[start..self.at]).unwrap();
                    match text.parse() {
                        Ok(n) => Ok(Value::Number(n)),
                        Err(_) => self.err("a bad number"),
                    }
                }
                _ => {
                    for word in ["true", "false", "null"] {
                        if self.s[self.at..].starts_with(word.as_bytes()) {
                            self.at += word.len();
                            return Ok(Value::Other);
                        }
                    }
                    self.err("expected a value")
                }
            }
        }

        fn string(&mut self) -> Result<String, String> {
            if self.s.get(self.at) != Some(&b'"') {
                return self.err("expected a string");
            }
            self.at += 1;
            let mut out = Vec::new();
            loop {
                match *self.s.get(self.at).ok_or("a string runs past the end")? {
                    b'"' => {
                        self.at += 1;
                        return String::from_utf8(out).map_err(|e| e.to_string());
                    }
                    b'\\' => {
                        let esc = *self.s.get(self.at + 1).ok_or("a string runs past the end")?;
                        self.at += 2;
                        let c = match esc {
                            b'"' => '"',
                            b'\\' => '\\',
                            b'/' => '/',
                            b'b' => '\u{8}',
                            b'f' => '\u{c}',
                            b'n' => '\n',
                            b'r' => '\r',
                            b't' => '\t',
                            // (A surrogate pair comes out as two U+FFFD: no name of ours has one.)
                            b'u' => {
                                let hex = self
                                    .s
                                    .get(self.at..self.at + 4)
                                    .and_then(|h| std::str::from_utf8(h).ok())
                                    .and_then(|h| u32::from_str_radix(h, 16).ok());
                                let Some(code) = hex else {
                                    return self.err("a bad \\u escape");
                                };
                                self.at += 4;
                                char::from_u32(code).unwrap_or('\u{fffd}')
                            }
                            _ => return self.err("a bad escape"),
                        };
                        out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                    }
                    b => {
                        out.push(b);
                        self.at += 1;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A safetensors file of `tensors` (name, shape, data), as `VarMap::save` writes one.
    fn file(tensors: &[(&str, &[usize], Vec<f32>)]) -> Vec<u8> {
        let mut header = String::from("{\"__metadata__\":{\"format\":\"pt\"}");
        let mut data = Vec::new();
        for (name, shape, values) in tensors {
            let start = data.len();
            data.extend(values.iter().flat_map(|v| v.to_le_bytes()));
            let shape: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            header += &format!(
                ",\"{name}\":{{\"dtype\":\"F32\",\"shape\":[{}],\"data_offsets\":[{start},{}]}}",
                shape.join(","),
                data.len()
            );
        }
        header += "}  ";
        let mut out = (header.len() as u64).to_le_bytes().to_vec();
        out.extend(header.as_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn one_layer_is_a_matrix_product() {
        // Weight row r is all r: logit r is r times the sum of the observation, plus r's bias.
        let weight: Vec<f32> = (0..LOGITS).flat_map(|r| [r as f32; OBS_LEN]).collect();
        let bias: Vec<f32> = (0..LOGITS).map(|r| 0.5 * r as f32).collect();
        let bytes = file(&[
            ("actor.l0.weight", &[LOGITS, OBS_LEN], weight),
            ("actor.l0.bias", &[LOGITS], bias),
            ("critic.l0.bias", &[1], vec![7.0]),
        ]);
        let actor = Actor::from_safetensors(&bytes).unwrap();
        let mut obs = [0.0; OBS_LEN];
        obs[0] = 1.0;
        obs[OBS_LEN - 1] = 0.25;
        let logits = actor.logits(&obs);
        for (r, l) in logits.iter().enumerate() {
            assert_eq!(*l, r as f32 * 1.25 + 0.5 * r as f32);
        }
        // The last of each head is the largest.
        let action = actor.act(&obs, true, &mut Rng::new(0));
        assert_eq!(action.heads(), HEADS.map(|n| n - 1));
    }

    #[test]
    fn a_wrong_observation_is_refused() {
        let bytes = file(&[
            ("actor.l0.weight", &[LOGITS, OBS_LEN + 1], vec![0.0; LOGITS * (OBS_LEN + 1)]),
            ("actor.l0.bias", &[LOGITS], vec![0.0; LOGITS]),
        ]);
        let err = Actor::from_safetensors(&bytes).err().unwrap();
        assert!(err.contains("another observation"), "{err}");
    }

    #[test]
    fn log_softmax_sums_to_one() {
        let lp = log_softmax(&[1.0, -2.0, 0.5, 3.0]);
        let sum: f32 = lp.iter().map(|l| l.exp()).sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }
}
