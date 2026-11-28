//! End-to-end smoke test for the update check + download/verify path,
//! against a real local HTTP server and a throwaway minisign key pair
//! generated fresh for this test run — never the real signing key kept
//! at `~/.tauri/linh-updater/`.
//!
//! `#[ignore]`d because it binds a local TCP port and is slower than the
//! rest of the suite; run explicitly with
//! `cargo test --test updater_e2e -- --ignored`. Phase 5 (CI) is where
//! this runs on every build, per the plan.
//!
//! Windows-only: this test binary links `tauri`/`wry` (via
//! `tauri::test::mock_app`) but, unlike `src/main.rs`, carries no
//! application manifest requesting comctl32 v6 — so on Windows the
//! executable itself fails to start (`STATUS_ENTRYPOINT_NOT_FOUND`,
//! missing `TaskDialogIndirect`) before any test, `#[ignore]`d or not,
//! ever runs. Compiled only on non-Windows until that's worth fixing
//! (phase 5's CI matrix only needs Linux/macOS/Windows package installs,
//! not this file).
//!
//! Also gated on the `self-update` feature: `tauri_plugin_updater` isn't
//! even a dependency with that feature off (the `msstore`/`mas`
//! `LINH_DIST` channels — see `Cargo.toml`), so this whole binary has
//! nothing to test there.
#![cfg(all(not(windows), feature = "self-update"))]

use base64::Engine;
use tauri_plugin_updater::UpdaterExt;

const PACKAGE_BYTES: &[u8] = b"linh-updater-e2e-fixture-package";
const TEST_TARGET: &str = "e2e-test-target";

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// Binds a `tiny_http` server on an ephemeral local port and returns it
/// together with its base URL, *before* serving anything — so the final
/// port (needed inside the manifest's download URL) is known before the
/// manifest body is built.
fn bind_server() -> (tiny_http::Server, String) {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("bind local test server");
    let base_url = format!("http://{}", server.server_addr());
    (server, base_url)
}

/// Serves `manifest_json` at `/latest.json` and the fixture package
/// bytes at `/package.bin` on a background thread for the rest of the
/// test process's life (no shutdown handle needed: the process exits
/// when the test does).
fn serve(server: tiny_http::Server, manifest_json: String) {
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let (body, content_type): (Vec<u8>, &str) = match request.url() {
                "/latest.json" => (manifest_json.clone().into_bytes(), "application/json"),
                "/package.bin" => (PACKAGE_BYTES.to_vec(), "application/octet-stream"),
                _ => (Vec::new(), "text/plain"),
            };
            let header =
                tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
                    .expect("valid header");
            let response = tiny_http::Response::from_data(body).with_header(header);
            let _ = request.respond(response);
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "binds a local TCP port; run explicitly with `-- --ignored`"]
async fn check_finds_a_newer_version_and_verifies_its_signature_against_a_local_server() {
    // A throwaway key pair, generated fresh for this run only.
    let keypair =
        minisign::KeyPair::generate_unencrypted_keypair().expect("generate throwaway keypair");
    let public_key_text = keypair.pk.to_box().expect("box public key").into_string();

    let signature_box = minisign::sign(
        None,
        &keypair.sk,
        PACKAGE_BYTES,
        Some("test fixture signature"),
        None,
    )
    .expect("sign the fixture package");
    let signature_text = signature_box.into_string();

    let (server, base_url) = bind_server();
    // Built by hand (not `serde_json::json!`) because the platform key is
    // the `TEST_TARGET` constant, not a literal the macro could spell.
    let manifest_json = format!(
        r#"{{
            "version": "9.9.9",
            "notes": "throwaway-key fixture release",
            "pub_date": "2026-01-01T00:00:00Z",
            "platforms": {{
                "{TEST_TARGET}": {{
                    "url": "{base_url}/package.bin",
                    "signature": "{}"
                }}
            }}
        }}"#,
        b64(&signature_text)
    );
    serve(server, manifest_json);

    // `mock_context` has no `plugins.updater` entry at all, and the
    // plugin's `Config::pubkey` has no `#[serde(default)]` — deserializing
    // its config fails before the builder below ever gets a chance to
    // override it, unless the context already carries *some* value for
    // it. A placeholder here is fine: `Builder::pubkey` below replaces it
    // with the real throwaway key before any request is made.
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context
        .config_mut()
        .plugins
        .0
        .insert("updater".into(), serde_json::json!({ "pubkey": "" }));

    let app = tauri::test::mock_builder()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(b64(&public_key_text))
                .target(TEST_TARGET)
                .build(),
        )
        .build(context)
        .expect("build mock app with the updater plugin registered");

    let handle = app.handle().clone();
    let updater = handle
        .updater_builder()
        .endpoints(vec![format!("{base_url}/latest.json").parse().unwrap()])
        .expect("http endpoint accepted on a debug build")
        .build()
        .expect("build updater");

    let update = updater
        .check()
        .await
        .expect("check succeeds against the local server")
        .expect("a newer version is found");

    assert_eq!(update.version, "9.9.9");
    assert_eq!(
        update.body.as_deref(),
        Some("throwaway-key fixture release")
    );

    // `download` verifies the downloaded bytes' signature against the
    // manifest's `signature` field and the plugin's configured pubkey
    // (here, the throwaway one above) before returning them.
    let downloaded = update
        .download(|_, _| {}, || {})
        .await
        .expect("download succeeds and the signature verifies");
    assert_eq!(downloaded, PACKAGE_BYTES);
}
