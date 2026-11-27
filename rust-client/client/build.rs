//! Builds the vendored libogg 1.3.5, libopus 1.3.1 (float) and opusfile 0.12 (the
//! versions in ddnet-libs) for the map sounds, with the `cc` crate (no CMake). The
//! configuration is ddnet-libs': float, x86 intrinsics with run-time CPU detection
//! (DECISIONS.md 13.3).

use std::path::Path;

fn main() {
    let vendor = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vendor");
    println!("cargo:rerun-if-changed={}", vendor.display());
    let (ogg, opus, opusfile) = (
        vendor.join("ogg"),
        vendor.join("opus"),
        vendor.join("opusfile"),
    );
    // opusfile first: GNU linkers resolve a static library only from the ones after it

    cc::Build::new()
        .include(opusfile.join("include"))
        .include(ogg.join("include"))
        .include(opus.join("include"))
        .files(
            ["opusfile.c", "info.c", "internal.c", "stream.c"]
                .map(|f| opusfile.join("src").join(f)),
        )
        .define("OP_DISABLE_HTTP", None)
        .define("OP_HAVE_LRINTF", None)
        .warnings(false)
        .compile("opusfile");

    let list = |name: &str| -> Vec<std::path::PathBuf> {
        let text = std::fs::read_to_string(opus.join(name)).expect("source list");
        text.lines()
            .filter(|l| !l.is_empty())
            .map(|f| opus.join(f))
            .collect()
    };
    // ddnet-libs' libopus is built with its x86 intrinsics and run-time CPU detection
    // (SSE and SSE2 presumed on x86-64, SSE4.1 detected); the float paths they pick decide
    // the decoded samples, so the same configuration is built here
    let x86 = matches!(
        std::env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        Ok("x86_64")
    );
    let opus_build = || {
        let mut b = cc::Build::new();
        b.includes(["include", "celt", "silk", "silk/float", ""].map(|d| opus.join(d)))
            .define("OPUS_BUILD", None)
            .define("USE_ALLOCA", None)
            .define("HAVE_LRINTF", None)
            .define("HAVE_LRINT", None)
            .warnings(false);
        if x86 {
            for d in [
                "OPUS_HAVE_RTCD",
                "CPU_INFO_BY_C",
                "OPUS_X86_MAY_HAVE_SSE",
                "OPUS_X86_PRESUME_SSE",
                "OPUS_X86_MAY_HAVE_SSE2",
                "OPUS_X86_PRESUME_SSE2",
                "OPUS_X86_MAY_HAVE_SSE4_1",
            ] {
                b.define(d, None);
            }
        }
        b
    };
    let mut files = list("sources.txt");
    if x86 {
        files.extend(list("sources_x86.txt"));
    }
    opus_build().files(files).compile("opus");
    if x86 {
        opus_build()
            .files(list("sources_sse4_1.txt"))
            .flag_if_supported("-msse4.1")
            .compile("opus_sse4_1");
    }

    cc::Build::new()
        .include(ogg.join("include"))
        .files(["src/bitwise.c", "src/framing.c"].map(|f| ogg.join(f)))
        .warnings(false)
        .compile("ogg");
}
