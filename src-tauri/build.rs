/// Every `LINH_DIST` value this build understands. Kept as the single
/// source of truth here (rather than duplicated into `updates.rs`) since
/// `updates::parse_dist` is checked against exactly this set by its own
/// tests.
const VALID_DISTS: &[&str] = &["github", "msstore", "mas", "flathub", "copr", "ppa"];

/// Store policies (Microsoft Store, Apple's App Store guideline 2.5.2)
/// forbid shipping a self-updating code path at all, so these two must
/// never be built with the `self-update` feature on.
const DISTS_FORBIDDING_SELF_UPDATE: &[&str] = &["msstore", "mas"];

fn main() {
    let dist = std::env::var("LINH_DIST").unwrap_or_else(|_| "github".to_string());
    println!("cargo:rerun-if-env-changed=LINH_DIST");

    if !VALID_DISTS.contains(&dist.as_str()) {
        panic!(
            "LINH_DIST={dist:?} is not a recognized distribution channel; expected one of {VALID_DISTS:?}"
        );
    }

    // Cargo sets `CARGO_FEATURE_<NAME>` for every enabled feature of the
    // package being built (its name upper-cased, `-` turned into `_`),
    // visible to this build script. See the `self-update` feature in
    // `Cargo.toml` (default-on) and its gating of `tauri-plugin-updater`
    // throughout `src/updates.rs`/`src/main.rs`.
    let self_update_enabled = std::env::var("CARGO_FEATURE_SELF_UPDATE").is_ok();
    if DISTS_FORBIDDING_SELF_UPDATE.contains(&dist.as_str()) && self_update_enabled {
        panic!(
            "LINH_DIST={dist} forbids the self-updater (Microsoft Store / Mac App Store policy \
             against self-updating code paths), but the default `self-update` feature is still \
             enabled. Build with `--no-default-features` so it's off. See docs/release.md for \
             the exact store build command."
        );
    }

    println!("cargo:rustc-env=LINH_DIST={dist}");

    tauri_build::build()
}
