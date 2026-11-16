# libxmp-lite 4.7.4 (vendored)

- Source: https://github.com/libxmp/libxmp/releases/download/libxmp-4.7.4/libxmp-lite-4.7.4.tar.gz
- SHA-256 of the release tarball: `d4b6108b6e8c7ecc106403bc4bab28fb46686867a7b5247567371ad557634c12`
- License: MIT (see README).
- Contents: `src/`, `include/` and `README` from the release, unmodified. Build files were left out: it is compiled by `crates/xmp-sys/build.rs` with `cc`, using the source list and defines from the release's `cmake/libxmp-sources.cmake` and `CMakeLists.txt` (`LIBXMP_CORE_PLAYER`, `LIBXMP_STATIC`).
- Used by the runtime's FMOD 3.61 layer to play the game's XM music.
