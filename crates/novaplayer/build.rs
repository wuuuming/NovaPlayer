//! Stages the bundled fonts next to the built program: NovaUI reads `fonts/` beside the
//! executable (Noto Sans + SC / JP / KR + Arabic / Hebrew / Devanagari / Thai, one variable file
//! each, OFL). They are not in the repository: the first build downloads them from Google
//! Fonts' and Noto's own repositories at pinned commits, checks each by sha256 and keeps them in
//! `<target>/novaplayer-downloads/fonts/`. Offline and without them, the program uses the
//! system's fonts.
//!
//! It also names what the About section shows: `NOVA_PLAYER_CORE_REV`, the Player-Core commit
//! this manifest pins, and `NOVA_COMMIT`, this repository's commit when built from a checkout.

use std::path::{Path, PathBuf};

/// The pinned Player-Core commit (the manifest's `rev`, a full hash, kept to its first seven
/// characters) and this checkout's commit.
fn revisions() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap_or_default();
    let core = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("player-core") && l.contains("rev"))
        .and_then(|l| l.split("rev").nth(1)?.split('"').nth(1).map(|r| r.chars().take(7).collect::<String>()))
        .unwrap_or_default();
    println!("cargo:rustc-env=NOVA_PLAYER_CORE_REV={core}");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    // Rebuilt when the checkout moves to another commit.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(dir);
        println!("cargo:rerun-if-changed={}", dir.join("HEAD").display());
        if let Some(head) = git(&["symbolic-ref", "-q", "HEAD"]) {
            // A packed branch has no file of its own: watch where it is packed.
            let file = Some(dir.join(head)).filter(|f| f.exists()).unwrap_or_else(|| dir.join("packed-refs"));
            println!("cargo:rerun-if-changed={}", file.display());
        }
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    println!("cargo:rustc-env=NOVA_COMMIT={commit}");
}

const GOOGLE_FONTS: &str = "https://raw.githubusercontent.com/google/fonts/45e3d5269e4a17d823f6d84470bc084dc5ae46f6/";
const NOTO: &str =
    "https://raw.githubusercontent.com/notofonts/notofonts.github.io/957448b6168f09dca2c8005449e9b6f3fd2dd207/";
/// (file, its address under `GOOGLE_FONTS` or `NOTO`, sha256).
const FONTS: [(&str, &str, &str, &str); 9] = [
    (
        "NotoSans-VF.ttf",
        GOOGLE_FONTS,
        "ofl/notosans/NotoSans%5Bwdth,wght%5D.ttf",
        "bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d",
    ),
    (
        "NotoSansSC-VF.ttf",
        GOOGLE_FONTS,
        "ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf",
        "a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da",
    ),
    (
        "NotoSansJP-VF.ttf",
        GOOGLE_FONTS,
        "ofl/notosansjp/NotoSansJP%5Bwght%5D.ttf",
        "c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f",
    ),
    (
        "NotoSansKR-VF.ttf",
        GOOGLE_FONTS,
        "ofl/notosanskr/NotoSansKR%5Bwght%5D.ttf",
        "194018e6b2b293a7964f037b25c0249ce1418bc9ab3c971060a03aa57861e252",
    ),
    (
        "NotoSansArabic-VF.ttf",
        NOTO,
        "fonts/NotoSansArabic/unhinted/variable-ttf/NotoSansArabic%5Bwdth,wght%5D.ttf",
        "3b735d0b7f04ddd5c6a3d7fafba8d8216420d3cbbba93ded80f34ba37369de00",
    ),
    (
        "NotoSansHebrew-VF.ttf",
        NOTO,
        "fonts/NotoSansHebrew/unhinted/variable-ttf/NotoSansHebrew%5Bwdth,wght%5D.ttf",
        "2a0af06d3e959ebc3f50a8c0042c24b671b6b3283d1ebbf3e7d39ef2b4ce62ba",
    ),
    (
        "NotoSansDevanagari-VF.ttf",
        NOTO,
        "fonts/NotoSansDevanagari/unhinted/variable-ttf/NotoSansDevanagari%5Bwdth,wght%5D.ttf",
        "9f82923465ef82fb86c6b99cbdd4947ba832fe3335948de815b18adfd75947c7",
    ),
    (
        "NotoSansThai-VF.ttf",
        NOTO,
        "fonts/NotoSansThai/unhinted/variable-ttf/NotoSansThai%5Bwdth,wght%5D.ttf",
        "70bcda1aae0976b35ff22660b918d0f610a4bd5c7136cf7313072c0fe0186825",
    ),
    (
        "OFL.txt",
        GOOGLE_FONTS,
        "ofl/notosans/OFL.txt",
        "cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a",
    ),
];

fn sha256_of(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    Some(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// `curl`: on Windows the system's own (System32), so another toolset's on PATH is not taken.
fn curl() -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        root.join("System32").join("curl.exe")
    } else {
        PathBuf::from("curl")
    }
}

/// `path` holding `url`'s bytes with the given sha256: kept when it already does, else
/// downloaded (to a file of this process's own, then moved into place). False when it could not
/// be had (offline): the build goes on without it.
fn fetch(url: &str, sha256: &str, path: &Path) -> bool {
    if sha256_of(path).as_deref() == Some(sha256) {
        return true;
    }
    let part = path.with_extension(format!("part{}", std::process::id()));
    let ok = std::process::Command::new(curl())
        .args(["-fsSL", "--retry", "3", "-o"])
        .arg(&part)
        .arg(url)
        .status()
        .is_ok_and(|s| s.success());
    let got = sha256_of(&part);
    if !ok || got.as_deref() != Some(sha256) {
        let _ = std::fs::remove_file(&part);
        println!("cargo:warning=could not get {url} (sha256 {got:?}, expected {sha256}); the program uses the system's fonts");
        return false;
    }
    std::fs::rename(&part, path).is_ok() || sha256_of(path).as_deref() == Some(sha256)
}

fn main() {
    revisions();
    // The backdrop's GPU program, compiled now (novaui-shaders): no shader compiler in the program.
    novaui_shaders::build_effect("field", "src/views/field.wgsl", "np_field");
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out: the program goes three levels up,
    // the downloads four.
    let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else { return };
    let (Some(bin), Some(target)) = (out.ancestors().nth(3), out.ancestors().nth(4)) else { return };
    let cache = target.join("novaplayer-downloads").join("fonts");
    let dst = bin.join("fonts");
    let _ = std::fs::create_dir_all(&cache);
    let _ = std::fs::create_dir_all(&dst);
    for (name, base, path, sha256) in FONTS {
        let from = cache.join(name);
        if !fetch(&format!("{base}{path}"), sha256, &from) {
            continue;
        }
        let to = dst.join(name);
        let same = std::fs::metadata(&from).ok().map(|m| m.len()) == std::fs::metadata(&to).ok().map(|m| m.len());
        if !same {
            if let Err(e) = std::fs::copy(&from, &to) {
                println!("cargo:warning=could not stage {name}: {e}");
            }
        }
    }
}
