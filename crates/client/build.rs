//! Records the build's version for `/version`: the nearest tag before the build's commit
//! (`git describe --tags --abbrev=0`), and the commit's short sha when the commit is not the
//! tag's. Without git, or a tag, there is the package's version only.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn main() {
    // A new commit, or a tag that moves, is a new version.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/refs");
        println!("cargo:rerun-if-changed={dir}/packed-refs");
    }
    println!("cargo:rerun-if-changed=build.rs");

    let tag = git(&["describe", "--tags", "--abbrev=0"]);
    let exact = git(&["describe", "--tags", "--exact-match"]).is_some();
    let sha = git(&["rev-parse", "--short", "HEAD"]);
    let version = match (tag, sha) {
        (Some(tag), _) if exact => tag,
        (Some(tag), Some(sha)) => format!("{tag} ({sha})"),
        (None, Some(sha)) => format!("{} ({sha})", env!("CARGO_PKG_VERSION")),
        (Some(tag), None) => tag,
        (None, None) => env!("CARGO_PKG_VERSION").to_string(),
    };
    println!("cargo:rustc-env=CTR_VERSION={version}");
}
