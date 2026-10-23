//! Differential test: run random inputs through this crate and through the
//! reference C implementation, and require identical results.
//!
//! The C side is built with `gcc -m32` from `reference/ctr-native`; the test
//! is skipped (with a note) if the reference is unavailable.
//! `MATH_REFERENCE_TRIALS` overrides the number of random trials per function.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use math::*;

/// Small deterministic xorshift64* generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// A signed value with a random magnitude (random bit width), so small and
    /// large operands are both common.
    fn signed(&mut self, max_bits: u32) -> i32 {
        let bits = self.below(max_bits as u64 + 1) as u32;
        let v = (self.next() >> 33) as i32 & (((1u64 << bits) - 1) as i32);
        if self.below(2) == 0 { -v } else { v }
    }
}

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/ctr-native")
}

/// The body of `.trigApprox = { ... }` in `zGlobal_DATA.c`.
fn trig_initialiser(source: &str) -> String {
    let at = source
        .find(".trigApprox")
        .expect("no .trigApprox in zGlobal_DATA.c");
    let open = at + source[at..].find('{').unwrap();
    let mut depth = 0;
    for (i, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[open + 1..open + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated .trigApprox initialiser");
}

fn build_reference_driver() -> Option<PathBuf> {
    let reference = reference_dir();
    if !reference.exists() {
        eprintln!("skipping: {} not found", reference.display());
        return None;
    }
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let inc = tmp.join("math_trig_approx.inc");
    let data_c = fs::read_to_string(reference.join("game/zGlobal_DATA.c")).unwrap();
    fs::write(&inc, trig_initialiser(&data_c)).unwrap();

    let exe = tmp.join("math_reference");
    let status = Command::new("gcc")
        .args(["-m32", "-DCTR_NATIVE", "-w", "-O1"])
        .arg(format!("-DTRIG_APPROX_INC=\"{}\"", inc.display()))
        .arg("-I")
        .arg(reference.join("include"))
        .arg("-I")
        .arg(reference.join("game"))
        .arg("-I")
        .arg(&reference)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c"))
        .args(
            [
                "platform/native_libgte.c",
                "platform/native_gte_core.c",
                "platform/native_inline_c.c",
                "game/MATH/MATH_0_Sin.c",
                "game/MATH/MATH_1_Cos.c",
                "game/MATH/MATH_2_FastSqrt.c",
                "game/MATH/MATH_4_VectorLength.c",
                "game/MATH/MATH_5_VectorNormalize.c",
                "game/MATH/MATH_7_MatrixStubs.c",
            ]
            .map(|f| reference.join(f)),
        )
        // regionsEXE.h defines `sdata` (identically) in every translation unit.
        .arg("-Wl,-z,muldefs")
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("failed to run gcc");
    assert!(
        status.success(),
        "could not build the C reference driver (needs gcc with -m32)"
    );
    Some(exe)
}

/// One query: the script line for the C driver and the Rust result in the same text form.
struct Case {
    script: String,
    rust: String,
}

fn case(script: String, rust: String) -> Case {
    Case { script, rust }
}

fn cases(trials: u64) -> Vec<Case> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut out = Vec::new();
    let mut add = |script: String, rust: String| out.push(case(script, rust));

    // Trig: every angle for two turns plus negatives, then random 32-bit angles.
    for a in -0x2000i32..0x2000 {
        add(format!("s {a}"), sin(a as u32).to_string());
        add(format!("c {a}"), cos(a as u32).to_string());
        add(format!("S {a}"), rsin(a).to_string());
        add(format!("C {a}"), rcos(a).to_string());
    }
    for _ in 0..trials {
        let a = rng.next() as i32;
        add(format!("s {a}"), sin(a as u32).to_string());
        add(format!("c {a}"), cos(a as u32).to_string());
        add(format!("S {a}"), rsin(a).to_string());
        add(format!("C {a}"), rcos(a).to_string());
    }

    // ratan2: axes, diagonals, then random magnitudes of every size.
    for y in -5i32..=5 {
        for x in -5i32..=5 {
            add(format!("a {y} {x}"), ratan2(y, x).to_string());
        }
    }
    for _ in 0..trials {
        let (y, x) = (rng.signed(31), rng.signed(31));
        add(format!("a {y} {x}"), ratan2(y, x).to_string());
    }

    // Square roots (non-negative; SquareRoot0 indexes its table unchecked).
    for v in 0i32..0x4000 {
        add(format!("q {v}"), square_root0(v).to_string());
        add(format!("Q {v}"), square_root0_stub(v).to_string());
    }
    for v in [i32::MAX, i32::MAX - 1, 0x4000_0000, 0x3fff_ffff] {
        add(format!("q {v}"), square_root0(v).to_string());
        add(format!("Q {v}"), square_root0_stub(v).to_string());
    }
    for _ in 0..trials {
        let v = rng.signed(31).abs();
        add(format!("q {v}"), square_root0(v).to_string());
        add(format!("Q {v}"), square_root0_stub(v).to_string());
        // The stub also accepts negative values.
        let n = -v;
        add(format!("Q {n}"), square_root0_stub(n).to_string());
        let (value, shift) = (rng.next() as u32, rng.below(63) as i32);
        add(
            format!("f {} {shift}", value as i32),
            fast_sqrt(value, shift).to_string(),
        );
    }

    // Vectors.
    for _ in 0..trials {
        let mut v = [0i16; 3];
        for c in &mut v {
            *c = match rng.below(4) {
                0 => rng.signed(15) as i16,
                1 => rng.next() as i16,
                2 => [i16::MIN, i16::MAX, 0, 1, -1, 0x1000][rng.below(6) as usize],
                _ => rng.signed(8) as i16,
            };
        }
        let sv = SVec3 {
            x: v[0],
            y: v[1],
            z: v[2],
        };
        add(
            format!("l {} {} {}", v[0], v[1], v[2]),
            vector_length(sv).to_string(),
        );
        let mut n = sv;
        let len = vector_normalize(&mut n);
        add(
            format!("n {} {} {}", v[0], v[1], v[2]),
            format!("{len} {} {} {}", n.x, n.y, n.z),
        );
    }
    out
}

#[test]
fn matches_reference_math() {
    let Some(exe) = build_reference_driver() else {
        return;
    };
    let trials: u64 = std::env::var("MATH_REFERENCE_TRIALS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);

    let cases = cases(trials);
    let mut script = String::new();
    for c in &cases {
        writeln!(script, "{}", c.script).unwrap();
    }
    let script_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("math_reference_script.txt");
    fs::write(&script_path, script).unwrap();

    let out = Command::new(&exe)
        .stdin(fs::File::open(&script_path).unwrap())
        .stdout(Stdio::piped())
        .output()
        .expect("failed to run the C reference driver");
    assert!(
        out.status.success(),
        "C reference driver failed: {}",
        out.status
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "C driver printed {} lines for {} cases",
        lines.len(),
        cases.len()
    );

    let mut failures = 0;
    for (c, got) in cases.iter().zip(lines) {
        if c.rust != got {
            failures += 1;
            if failures <= 20 {
                eprintln!(
                    "MISMATCH `{}`: rust {} vs reference {got}",
                    c.script, c.rust
                );
            }
        }
    }
    assert_eq!(
        failures,
        0,
        "{failures} of {} cases differ from the reference",
        cases.len()
    );
}
