//! Compile the vendored libxmp-lite (third_party/libxmp-lite, see VENDORED.md)
//! with the release's source list and defines.

use std::path::PathBuf;

const SOURCES: &[&str] = &[
    "virtual.c", "format.c", "period.c", "player.c", "read_event.c", "misc.c", "dataio.c", "lfo.c",
    "scan.c", "control.c", "filter.c", "effects.c", "flow.c", "mixer.c", "mix_all.c", "load_helpers.c",
    "load.c", "filetype.c", "hio.c", "smix.c", "memio.c", "rng.c", "win32.c",
    "loaders/common.c", "loaders/itsex.c", "loaders/sample.c", "loaders/xm_load.c", "loaders/mod_load.c",
    "loaders/s3m_load.c", "loaders/it_load.c",
];

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../third_party/libxmp-lite");
    let mut b = cc::Build::new();
    for s in SOURCES {
        let p = root.join("src").join(s);
        println!("cargo:rerun-if-changed={}", p.display());
        b.file(p);
    }
    b.include(root.join("include/libxmp-lite"))
        .include(root.join("src"))
        .define("LIBXMP_CORE_PLAYER", None)
        .define("LIBXMP_STATIC", None)
        .warnings(false);
    if !b.get_compiler().is_like_msvc() {
        b.flag_if_supported("-std=gnu99");
    }
    b.compile("xmp_lite");
}
