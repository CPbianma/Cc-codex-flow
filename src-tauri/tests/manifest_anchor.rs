//! This is intentionally a near-empty integration-test binary whose only
//! purpose is to make `cargo` register a `[[test]]` target so that
//! `build.rs` can emit `cargo:rustc-link-arg-tests=...` directives — those
//! directives are only honoured when at least one test target exists, and
//! we need them to embed a Common-Controls v6 manifest into unit-test
//! binaries on Windows (otherwise `tauri-plugin-dialog` -> `comctl32`
//! triggers STATUS_ENTRYPOINT_NOT_FOUND on startup).
//!
//! `harness = false` keeps this from being picked up by `cargo test` as a
//! real test runner; we just need an empty `main` that exits 0.

fn main() {}
