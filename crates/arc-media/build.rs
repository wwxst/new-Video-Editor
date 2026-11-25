//! Builds the libav shim (csrc/arc_av.c) and links FFmpeg (docs/04 §1, docs/11 §2: LGPL, dynamic).
//!
//! FFmpeg is located via `ARCLINE_FFMPEG_DIR` (a directory with `include/`, `lib/`, `bin/`), falling
//! back to `pkg-config`. The version is pinned to the FFmpeg 9.1 package (spec
//! docs/specs/ffmpeg-9.1-upgrade.md §4.3/§4.7): the script gives the readable error and the shim's
//! `#error` is the backstop.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// `(library, major, minor, micro)` the shim requires. Later 9.x minors are accepted.
const REQUIRED: [(&str, u32, u32, u32); 5] = [
    ("avutil", 61, 19, 100),
    ("avcodec", 63, 24, 100),
    ("avformat", 63, 16, 100),
    ("swscale", 10, 5, 100),
    ("swresample", 7, 6, 100),
];

fn check_version(lib: &str, found: (u32, u32, u32), req: (u32, u32, u32)) -> Result<(), String> {
    let (ma, mi, mc) = found;
    let (ra, ri, rc) = req;
    if ma != ra {
        return Err(format!(
            "{lib} {ma}.{mi}.{mc} is not FFmpeg 9.x (required major {ra})"
        ));
    }
    if (mi, mc) < (ri, rc) {
        return Err(format!(
            "{lib} {ma}.{mi}.{mc} is older than the pinned {ra}.{ri}.{rc} (FFmpeg 9.1)"
        ));
    }
    Ok(())
}

/// Parse `LIB<NAME>_VERSION_{MAJOR,MINOR,MICRO}` from the installed headers.
fn parse_versions(dir: &Path, lib: &str) -> Result<(u32, u32, u32), String> {
    let base = dir.join("include").join(format!("lib{lib}"));
    let mut found: [Option<u32>; 3] = [None; 3];
    for file in ["version_major.h", "version.h"] {
        let p = base.join(file);
        println!("cargo:rerun-if-changed={}", p.display());
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let prefix = format!("LIB{}_VERSION", lib.to_ascii_uppercase());
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("#define ") else {
                continue;
            };
            let mut it = rest.split_whitespace();
            let (Some(name), Some(value)) = (it.next(), it.next()) else {
                continue;
            };
            for (suffix, slot) in [("_MAJOR", 0), ("_MINOR", 1), ("_MICRO", 2)] {
                if name == format!("{prefix}{suffix}") {
                    found[slot] = value.parse().ok();
                }
            }
        }
    }
    match found {
        [Some(a), Some(b), Some(c)] => Ok((a, b, c)),
        _ => Err(format!(
            "cannot read LIB{}_VERSION_* from {} (is this an FFmpeg development package?)",
            lib.to_ascii_uppercase(),
            base.display()
        )),
    }
}

fn read_ffversion(dir: &Path) -> Option<String> {
    let p = dir.join("include/libavutil/ffversion.h");
    println!("cargo:rerun-if-changed={}", p.display());
    let text = std::fs::read_to_string(p).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("#define FFMPEG_VERSION ") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn manifest_sha256() -> Option<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let p = root.join("../../third_party/ffmpeg/manifest.json");
    println!("cargo:rerun-if-changed={}", p.display());
    let bytes = std::fs::read(p).ok()?;
    let digest = Sha256::digest(&bytes);
    Some(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn main() {
    println!("cargo:rerun-if-changed=csrc/arc_av.c");
    println!("cargo:rerun-if-env-changed=ARCLINE_FFMPEG_DIR");
    let libs = ["avformat", "avcodec", "avutil", "swscale", "swresample"];
    let mut include: Vec<PathBuf> = Vec::new();
    let mut build_ident = "unpinned".to_string();

    if let Ok(dir) = std::env::var("ARCLINE_FFMPEG_DIR") {
        let dir = PathBuf::from(dir);
        for (lib, ma, mi, mc) in REQUIRED {
            let found = parse_versions(&dir, lib).unwrap_or_else(|e| {
                panic!(
                    "arc-media: {e}.\nSet ARCLINE_FFMPEG_DIR to the FFmpeg 9.1 package \
                     (see docs/specs/ffmpeg-9.1-upgrade.md §5 and HANDOVER.md)."
                )
            });
            check_version(lib, found, (ma, mi, mc)).unwrap_or_else(|e| {
                panic!(
                    "arc-media: {e}.\nSet ARCLINE_FFMPEG_DIR to the FFmpeg 9.1 package \
                     (see docs/specs/ffmpeg-9.1-upgrade.md §5 and HANDOVER.md)."
                )
            });
        }
        include.push(dir.join("include"));
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("lib").display()
        );
        for l in libs {
            println!("cargo:rustc-link-lib=dylib={l}");
            println!(
                "cargo:rerun-if-changed={}",
                dir.join("lib").join(format!("lib{l}.dll.a")).display()
            );
        }
        if let Some(v) = read_ffversion(&dir) {
            build_ident = v;
        }
    } else {
        let mut found = true;
        for (lib, ma, mi, mc) in REQUIRED {
            let name = format!("lib{lib}");
            match pkg_config::Config::new().probe(&name) {
                Ok(l) => {
                    let parts: Vec<u32> = l
                        .version
                        .split('.')
                        .filter_map(|x| x.parse().ok())
                        .collect();
                    if parts.len() < 3 {
                        panic!("arc-media: pkg-config {name} has an unparseable version {:?}", l.version);
                    }
                    check_version(lib, (parts[0], parts[1], parts[2]), (ma, mi, mc))
                        .unwrap_or_else(|e| panic!("arc-media: {e}"));
                    include.extend(l.include_paths);
                }
                Err(_) => found = false,
            }
        }
        if !found {
            panic!(
                "arc-media: FFmpeg 9.1 development files not found. Set ARCLINE_FFMPEG_DIR to the \
                 FFmpeg 9.1 package with include/ and lib/ (see docs/specs/ffmpeg-9.1-upgrade.md)."
            );
        }
    }
    println!("cargo:rustc-env=ARC_FFMPEG_BUILD_IDENT={build_ident}");
    println!(
        "cargo:rustc-env=ARC_FFMPEG_MANIFEST_SHA256={}",
        manifest_sha256().unwrap_or_else(|| "unpinned".to_string())
    );

    let mut b = cc::Build::new();
    b.file("csrc/arc_av.c")
        .warnings(true)
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Werror=deprecated-declarations");
    for i in &include {
        b.include(i);
    }
    b.compile("arc_av");
}
