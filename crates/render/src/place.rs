//! The driver's place in big digits, at the bottom left of a race: `big1` .. `big8`, the headers
//! of model 0x38 of a racer's pack. They are flat models with no texture, in the colours of their
//! corners: yellow at the top to red at the bottom, with a dark rim. Each is drawn here into a
//! picture once, for a UI to show as an image, with the suffix of the place after it in the
//! large font.

use std::io;

use loader::dram::DramFile;
use loader::model::{ModelHeader, parse_pack_models};

use crate::mesh::Atlas;

/// `STATIC_BIG1`
pub const MODEL: i16 = 0x38;

/// Texels of a picture for a unit of the model. The pictures are drawn larger than they are shown
/// on a PS1 screen, so that their edges stay smooth when the UI scales them.
pub const TEXELS_PER_UNIT: usize = 4;

/// Samples across and down in a texel, for the edges.
const SAMPLES: usize = 4;

/// A digit's picture, with a clear texel all round.
pub struct Digit {
    pub atlas: Atlas,
}

impl Digit {
    /// The picture's size in model units.
    pub fn size(&self) -> [f32; 2] {
        [self.atlas.width, self.atlas.height].map(|n| n as f32 / TEXELS_PER_UNIT as f32)
    }
}

/// The digits 1 to 8 from a racer's pack.
pub fn digits(pack: &DramFile) -> io::Result<Vec<Digit>> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let model = parse_pack_models(pack)?
        .into_iter()
        .find(|m| m.id == MODEL)
        .ok_or_else(|| invalid("the pack has no big1"))?;
    model
        .headers
        .iter()
        .map(|header| digit(header).ok_or_else(|| invalid("a big digit has no frame")))
        .collect()
}

/// The suffix of a place (1 is 1st), in capitals as the HUD writes it.
pub fn suffix(place: usize) -> &'static str {
    match (place % 10, place % 100) {
        (_, 11..=13) => "TH",
        (1, _) => "ST",
        (2, _) => "ND",
        (3, _) => "RD",
        _ => "TH",
    }
}

/// A corner of a triangle in samples (y down), and its colour.
type Corner = ([f32; 2], [f32; 3]);

fn digit(header: &ModelHeader) -> Option<Digit> {
    let frame = header
        .animations
        .first()
        .and_then(|a| a.frames.first())
        .or(header.static_frame.as_ref())?;
    let scale = [0, 1].map(|a| header.scale[a] as f32 / 4096.0);
    let triangles: Vec<[([f32; 2], [f32; 3]); 3]> = header
        .triangles(frame)
        .iter()
        .map(|t| {
            std::array::from_fn(|i| {
                let [x, y, _] = t.positions[i];
                let c = t.colors[i];
                (
                    [x as f32 * scale[0], y as f32 * scale[1]],
                    [c & 0xff, c >> 8 & 0xff, c >> 16 & 0xff].map(|v| v as f32),
                )
            })
        })
        .collect();
    let corners = || triangles.iter().flatten().map(|(p, _)| *p);
    let low = corners().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
    let high = corners().fold([f32::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
    // A clear texel all round, so that the edges blend with nothing.
    let size = [0, 1].map(|a| ((high[a] - low[a]) * TEXELS_PER_UNIT as f32).ceil() as usize + 2);
    let samples = [size[0] * SAMPLES, size[1] * SAMPLES];
    // Model y is up, the picture's down.
    let to_samples = |[x, y]: [f32; 2]| {
        let unit = (TEXELS_PER_UNIT * SAMPLES) as f32;
        [
            (x - low[0]) * unit + SAMPLES as f32,
            (high[1] - y) * unit + SAMPLES as f32,
        ]
    };
    let mut drawn: Vec<Option<[f32; 3]>> = vec![None; samples[0] * samples[1]];
    // The faces are flat and all at the same depth: each is drawn over the ones before it.
    for triangle in &triangles {
        let corners: [Corner; 3] = triangle.map(|(p, c)| (to_samples(p), c));
        fill(&mut drawn, samples[0], &corners);
    }
    let mut rgba = vec![0u8; size[0] * size[1] * 4];
    for ty in 0..size[1] {
        for tx in 0..size[0] {
            let (mut sum, mut covered) = ([0.0f32; 3], 0);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let at = (ty * SAMPLES + sy) * samples[0] + tx * SAMPLES + sx;
                    if let Some(c) = drawn[at] {
                        sum = [0, 1, 2].map(|i| sum[i] + c[i]);
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let texel = &mut rgba[(ty * size[0] + tx) * 4..][..4];
            for i in 0..3 {
                texel[i] = (sum[i] / covered as f32).round().min(255.0) as u8;
            }
            texel[3] = (covered * 255 / (SAMPLES * SAMPLES)) as u8;
        }
    }
    Some(Digit {
        atlas: Atlas {
            width: size[0],
            height: size[1],
            rgba,
        },
    })
}

/// Draws a triangle into `drawn` (`width` samples a row), its colour blended between the corners'
/// (Gouraud shading). A sample is in the triangle when its middle is, edges included.
fn fill(drawn: &mut [Option<[f32; 3]>], width: usize, corners: &[Corner; 3]) {
    let [(a, ca), (b, cb), (c, cc)] = *corners;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if area == 0.0 {
        return;
    }
    let height = drawn.len() / width;
    let span = |i: usize| {
        let (lo, hi) = (a[i].min(b[i]).min(c[i]), a[i].max(b[i]).max(c[i]));
        let limit = if i == 0 { width } else { height };
        (
            lo.floor().max(0.0) as usize,
            (hi.ceil() as usize).min(limit),
        )
    };
    let ((x0, x1), (y0, y1)) = (span(0), span(1));
    for y in y0..y1 {
        for x in x0..x1 {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let edge = |u: [f32; 2], v: [f32; 2]| {
                ((v[0] - u[0]) * (p[1] - u[1]) - (p[0] - u[0]) * (v[1] - u[1])) / area
            };
            let weights = [edge(b, c), edge(c, a), edge(a, b)];
            if weights.iter().any(|&w| w < 0.0) {
                continue;
            }
            drawn[y * width + x] = Some(
                [0, 1, 2].map(|i| ca[i] * weights[0] + cb[i] * weights[1] + cc[i] * weights[2]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loader::bigindex::BigIndex;
    use loader::disc::open_bigfile;

    #[test]
    fn places_have_their_suffixes() {
        let suffixes: Vec<_> = (1..=8).map(suffix).collect();
        assert_eq!(suffixes, ["ST", "ND", "RD", "TH", "TH", "TH", "TH", "TH"]);
        assert_eq!(suffix(11), "TH");
        assert_eq!(suffix(21), "ST");
    }

    #[test]
    fn digits_decode_from_the_disc() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ctr-u.bin");
        let Ok((mut disc, big)) = open_bigfile(&path) else {
            return;
        };
        let pack = big.read(&mut disc, BigIndex::ARCADE_1P_PACK.get()).unwrap();
        let digits = digits(&DramFile::parse(&pack).unwrap()).unwrap();
        assert_eq!(digits.len(), 8);
        for (n, digit) in digits.iter().enumerate() {
            let atlas = &digit.atlas;
            // About as tall as the models are (72 units), and a good part of the picture.
            let [width, height] = digit.size();
            assert!(
                (40.0..80.0).contains(&height),
                "{}: {width} by {height}",
                n + 1
            );
            let texels = atlas.rgba.chunks(4).filter(|p| p[3] == 255).count();
            assert!(
                texels > atlas.width * atlas.height / 4,
                "{}: {texels}",
                n + 1
            );
            // Yellow at the top, red at the bottom: the mean green over red of the opaque texels
            // of the top and the bottom quarter.
            let green = |rows: std::ops::Range<usize>| {
                let (mut g, mut r) = (0u64, 0u64);
                for y in rows {
                    for p in atlas.rgba[y * atlas.width * 4..][..atlas.width * 4].chunks(4) {
                        if p[3] == 255 && p[0] > 0x80 {
                            (g, r) = (g + p[1] as u64, r + p[0] as u64);
                        }
                    }
                }
                g as f32 / r.max(1) as f32
            };
            let quarter = atlas.height / 4;
            let (top, bottom) = (
                green(0..quarter),
                green(atlas.height - quarter..atlas.height),
            );
            assert!(top > bottom, "{}: top {top}, bottom {bottom}", n + 1);
        }
    }
}
