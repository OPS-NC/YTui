//! Embeds bundle/<target>/{ffmpeg,qjs} when they exist (built by
//! scripts/build-bundle.sh). Without them the binary still builds and relies
//! on the system's ffmpeg and JS runtime.

use std::path::Path;

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3))
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(bundled)");
    let target = std::env::var("TARGET").expect("TARGET");
    let root = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let dir = Path::new(&root).join("bundle").join(&target);
    let (ffmpeg, qjs) = (dir.join("ffmpeg"), dir.join("qjs"));
    println!("cargo::rerun-if-changed={}", dir.display());
    println!("cargo::rerun-if-changed={}", ffmpeg.display());
    println!("cargo::rerun-if-changed={}", qjs.display());
    let (Ok(a), Ok(b)) = (std::fs::read(&ffmpeg), std::fs::read(&qjs)) else { return };
    println!("cargo::rustc-cfg=bundled");
    println!("cargo::rustc-env=YTUI_BUNDLE_FFMPEG={}", ffmpeg.display());
    println!("cargo::rustc-env=YTUI_BUNDLE_QJS={}", qjs.display());
    // Versions the extracted copies: a new build re-extracts, an unchanged
    // one never rewrites them.
    println!("cargo::rustc-env=YTUI_BUNDLE_ID={:016x}", fnv1a(&a) ^ fnv1a(&b).rotate_left(17));
}
