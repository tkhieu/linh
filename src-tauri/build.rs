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

    write_dist_stamp(&dist, self_update_enabled);

    tauri_build::build()
}

/// Writes a small stamp file next to where Cargo will place the final
/// compiled binary (same directory, `linh.dist-marker.txt`), naming this
/// build's `LINH_DIST`/`self-update` state in plain text.
/// `scripts/package-msix.ps1` reads this to confirm which build it's
/// about to package into the Store MSIX before doing so.
///
/// This is the third approach tried for that check, and the only one
/// that held up: running the built exe with a `--print-dist` flag
/// crashed in real CI (`linh.exe` is a GUI-subsystem binary —
/// `windows_subsystem = "windows"` — with no reliable stdout when
/// spawned non-interactively: "failed printing to stdout: The pipe is
/// being closed"). Embedding a marker directly in the exe's data section
/// via a `#[used] static` (so a packaging script could search the raw
/// bytes without running it) also failed in real CI: it worked on the
/// Linux dev build this was first verified against, but the MSVC
/// linker's default `/OPT:REF` silently stripped the unreferenced static
/// from the real Windows build. A plain sibling file written by the
/// build script itself sidesteps both: nothing has to run or link
/// anything to produce or read it.
///
/// The format string here (`"LINH_BUILD_MARKER:dist={dist};self_update={self_update_enabled}\n"`)
/// must be kept identical to `updates::dist_stamp_contents` by hand —
/// build scripts compile separately from (and before) the crate they
/// build, so this can't call that function directly. That function's own
/// unit test is what a future change to either copy needs to keep
/// passing.
///
/// Derives the binary's directory from `OUT_DIR`
/// (`<target-dir>/<profile>/build/<pkg>-<hash>/out`, Cargo's own
/// documented convention) by finding the ancestor directory literally
/// named `build` and taking *its* parent — rather than walking up a
/// fixed number of levels, which turned out not to be robust: in real
/// CI, `OUT_DIR`'s actual depth below `build/` didn't match this
/// crate's local structure (observed landing one level too deep, inside
/// `build/` itself, across every platform), for a reason not fully
/// root-caused. Searching for the `build` component by name instead of
/// counting levels is self-correcting regardless of that extra nesting.
/// Best-effort throughout: a failure here (an unexpected `OUT_DIR`
/// shape with no `build` ancestor, a write error) must never fail the
/// actual build, since this stamp only serves `package-msix.ps1`,
/// nothing the compiled binary itself depends on.
fn write_dist_stamp(dist: &str, self_update_enabled: bool) {
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        println!("cargo:warning=write_dist_stamp: OUT_DIR is not set, skipping stamp file");
        return;
    };
    let out_path = std::path::Path::new(&out_dir);
    let build_dir = out_path
        .ancestors()
        .find(|a| a.file_name() == Some(std::ffi::OsStr::new("build")));
    let Some(profile_dir) = build_dir.and_then(std::path::Path::parent) else {
        println!(
            "cargo:warning=write_dist_stamp: could not find a 'build' ancestor (or its parent) in OUT_DIR ({out_dir}), skipping stamp file"
        );
        return;
    };
    let stamp_path = profile_dir.join("linh.dist-marker.txt");
    let contents = format!("LINH_BUILD_MARKER:dist={dist};self_update={self_update_enabled}\n");
    if let Err(err) = std::fs::write(&stamp_path, &contents) {
        println!(
            "cargo:warning=write_dist_stamp: failed to write {}: {err}",
            stamp_path.display()
        );
    }
}
