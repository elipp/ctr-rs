//! Records the build's version for `/version`: the nearest tag before the build's commit
//! (`git describe --tags --abbrev=0`), and the commit's short sha when the commit is not the
//! tag's. Without git, or a tag, there is the package's version only.
//!
//! On Windows it also puts `icon.png` in the executable, as its icon.

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

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_icon();
    }
}

/// An `.ico` of one image can hold a PNG as it is (since Vista), so the icon needs no converting.
#[cfg(windows)]
fn windows_icon() {
    println!("cargo:rerun-if-changed=icon.png");
    let png = std::fs::read("icon.png").unwrap();
    let mut ico = Vec::new();
    ico.extend([0, 0, 1, 0, 1, 0]);
    // 0 is 256 pixels wide and high.
    ico.extend([0, 0, 0, 0, 1, 0, 32, 0]);
    ico.extend((png.len() as u32).to_le_bytes());
    ico.extend(22u32.to_le_bytes());
    ico.extend(png);
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("icon.ico");
    std::fs::write(&path, ico).unwrap();
    winresource::WindowsResource::new()
        .set_icon(path.to_str().unwrap())
        .compile()
        .unwrap();
}
