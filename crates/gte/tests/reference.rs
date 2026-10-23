//! Differential test: run random register/command streams through this crate
//! and through the reference C implementation, and require identical state.
//!
//! The C side is built with `gcc -m32` from `reference/ctr-native`; the test
//! is skipped (with a note) if the reference or a 32-bit gcc is unavailable.
//! `GTE_REFERENCE_TRIALS` overrides the number of trials.

use std::fmt::Write as _;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gte::{Gte, cmd, ctrl};

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
    /// Register values biased towards the interesting ranges: zero, +-1.0 in
    /// 4.12, 16-bit extremes, small packed pairs, and plain noise.
    fn value(&mut self) -> u32 {
        let small = |r: &mut Self| (r.below(0x2001) as i32 - 0x1000) as u32;
        match self.below(9) {
            0 => 0,
            1 => 0x1000,
            2 => [
                0x7fff,
                0x8000,
                0xffff,
                0x7fff_ffff,
                0x8000_0000,
                0xffff_ffff,
            ][self.below(6) as usize],
            3 => small(self) & 0xffff | small(self) << 16,
            4 => small(self),
            5 => (self.next() as u32) & 0x0001_ffff,
            6 => (self.next() as u32) & 0xffff,
            _ => self.next() as u32,
        }
    }
}

/// Function codes the reference does not implement; both sides must reject them alike.
const UNIMPLEMENTED: [u32; 3] = [0x02, 0x1f, 0x3c];

fn random_funct(rng: &mut Rng) -> u32 {
    let implemented = cmd::ALL.len() as u64;
    match rng.below(implemented + UNIMPLEMENTED.len() as u64) {
        i if i < implemented => cmd::ALL[i as usize],
        i => UNIMPLEMENTED[(i - implemented) as usize],
    }
}

enum Cmd {
    Mtc2(usize, u32),
    Ctc2(usize, u32),
    Mfc2(usize),
    Cfc2(usize),
    Cop2(u32),
}

fn script(trials: u64) -> Vec<Cmd> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut cmds = Vec::new();
    for trial in 0..trials {
        // Occasionally randomise everything; otherwise perturb a few registers,
        // so that results of earlier commands feed into later ones.
        let (nd, nc) = if trial % 8 == 0 { (32, 32) } else { (4, 4) };
        for _ in 0..nd {
            let reg = if nd == 32 {
                cmds.len() % 32
            } else {
                rng.below(32) as usize
            };
            cmds.push(Cmd::Mtc2(reg, rng.value()));
        }
        for _ in 0..nc {
            cmds.push(Cmd::Ctc2(rng.below(32) as usize, rng.value()));
        }
        // A command clears FLAG, so its CTC2 behaviour is only visible if read straight back.
        cmds.push(Cmd::Ctc2(ctrl::FLAG, rng.value()));
        cmds.push(Cmd::Cfc2(ctrl::FLAG));
        cmds.push(Cmd::Cfc2(rng.below(32) as usize));
        let funct = random_funct(&mut rng);
        let operands = (rng.next() as u32) & cmd::OPERAND_MASK & !cmd::FUNCT_MASK;
        cmds.push(Cmd::Cop2(operands | funct));
        for _ in 0..3 {
            cmds.push(Cmd::Mfc2(rng.below(32) as usize));
        }
    }
    cmds
}

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/ctr-native")
}

/// Builds the C driver. Returns `None` only when the reference checkout is
/// absent; if it is there, a failed build is a test failure, not a skip.
fn build_reference_driver() -> Option<PathBuf> {
    let r = reference_dir();
    if !r.join("platform/native_gte_core.c").exists() {
        eprintln!("skipping: reference/ctr-native not found");
        return None;
    }
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gte_reference");
    let status = Command::new("gcc")
        .args(["-m32", "-O1", "-DCTR_NATIVE", "-I"])
        .arg(r.join("include"))
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference/reference.c"))
        .arg(r.join("platform/native_gte_core.c"))
        .arg(r.join("platform/native_inline_c.c"))
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

fn dump(g: &Gte, ret: bool) -> String {
    let mut s = format!("{}", ret as i32);
    for w in g.data_regs().iter().chain(g.ctrl_regs()) {
        write!(s, " {w:x}").unwrap();
    }
    s
}

#[test]
fn matches_reference_gte() {
    let Some(exe) = build_reference_driver() else {
        return;
    };
    let trials: u64 = std::env::var("GTE_REFERENCE_TRIALS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let cmds = script(trials);

    // Feed the C side from a file so a large script can't deadlock the pipe.
    let script_path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gte_reference_script.txt");
    let mut text = String::new();
    for c in &cmds {
        match c {
            Cmd::Mtc2(r, v) => writeln!(text, "m {r} {v}"),
            Cmd::Ctc2(r, v) => writeln!(text, "c {r} {v}"),
            Cmd::Mfc2(r) => writeln!(text, "r {r}"),
            Cmd::Cfc2(r) => writeln!(text, "f {r}"),
            Cmd::Cop2(op) => writeln!(text, "x {op}"),
        }
        .unwrap();
    }
    std::fs::write(&script_path, text).unwrap();
    let out = Command::new(&exe)
        .stdin(File::open(&script_path).unwrap())
        .stdout(Stdio::piped())
        .output()
        .expect("run reference driver");
    assert!(out.status.success(), "reference driver crashed");
    let expected = String::from_utf8(out.stdout).unwrap();
    let mut expected = expected.lines();

    let mut gte = Gte::new();
    for (i, c) in cmds.iter().enumerate() {
        match c {
            Cmd::Mtc2(r, v) => gte.mtc2(*v, *r),
            Cmd::Ctc2(r, v) => gte.ctc2(*v, *r),
            Cmd::Mfc2(r) => {
                let want = expected.next().expect("reference output ended early");
                let got = gte.mfc2(*r).to_string();
                assert_eq!(got, want, "command {i}: mfc2 r{r}");
            }
            Cmd::Cfc2(r) => {
                let want = expected.next().expect("reference output ended early");
                let got = gte.cfc2(*r).to_string();
                assert_eq!(got, want, "command {i}: cfc2 r{r}");
            }
            Cmd::Cop2(op) => {
                let want = expected.next().expect("reference output ended early");
                let ret = gte.cop2(*op);
                let got = dump(&gte, ret);
                if got != want {
                    let (g, w): (Vec<_>, Vec<_>) =
                        (got.split(' ').collect(), want.split(' ').collect());
                    let diffs: Vec<String> = (0..g.len().min(w.len()))
                        .filter(|&k| g[k] != w[k])
                        .map(|k| {
                            let name = match k {
                                0 => "ret".to_string(),
                                1..=32 => format!("d{}", k - 1),
                                _ => format!("c{}", k - 33),
                            };
                            format!("{name}: rust {} / ref {}", g[k], w[k])
                        })
                        .collect();
                    panic!(
                        "command {i}: cop2 {op:#x} (funct {:#x}) differs:\n  {}",
                        op & cmd::FUNCT_MASK,
                        diffs.join("\n  ")
                    );
                }
            }
        }
    }
    assert!(expected.next().is_none(), "reference produced extra output");
    println!(
        "{} commands over {trials} trials matched the reference",
        cmds.len()
    );
}
