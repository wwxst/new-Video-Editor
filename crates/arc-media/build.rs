//! Builds the libav shim (csrc/arc_av.c) and links FFmpeg (docs/04 §1, docs/11 §2: LGPL, dynamic).
//!
//! FFmpeg is located via `ARCLINE_FFMPEG_DIR` (a directory with `include/`, `lib/`, `bin/`), falling
//! back to `pkg-config`. Use an LGPL build; `scripts/dev-env.sh` sets the variable.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=csrc/arc_av.c");
    println!("cargo:rerun-if-env-changed=ARCLINE_FFMPEG_DIR");
    let libs = ["avformat", "avcodec", "avutil", "swscale", "swresample"];
    let mut include: Vec<PathBuf> = Vec::new();

    if let Ok(dir) = std::env::var("ARCLINE_FFMPEG_DIR") {
        let dir = PathBuf::from(dir);
        include.push(dir.join("include"));
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("lib").display()
        );
        for l in libs {
            println!("cargo:rustc-link-lib=dylib={l}");
        }
    } else {
        let mut found = true;
        for l in libs {
            match pkg_config::Config::new()
                .atleast_version("0")
                .probe(&format!("lib{l}"))
            {
                Ok(lib) => include.extend(lib.include_paths),
                Err(_) => found = false,
            }
        }
        if !found {
            panic!(
                "arc-media: FFmpeg development files not found. Set ARCLINE_FFMPEG_DIR to an LGPL \
                 FFmpeg build with include/ and lib/ (see HANDOVER.md), or install pkg-config files."
            );
        }
    }

    let mut b = cc::Build::new();
    b.file("csrc/arc_av.c")
        .warnings(true)
        .flag_if_supported("-std=c11");
    for i in &include {
        b.include(i);
    }
    b.compile("arc_av");
}
