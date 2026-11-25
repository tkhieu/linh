# Contributing

Install the Fedora build dependencies and Rust, Node.js, and pnpm as described
in the [README](README.md#build-on-fedora-44). Then run:

```sh
cargo install tauri-cli --version '^2' --locked
pnpm --dir ui install
cd src-tauri && cargo tauri dev
```

Before opening a pull request, run these checks from the repository root:

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm --dir ui lint
pnpm --dir ui typecheck
pnpm --dir ui test
```

Use Conventional Commits for commit messages (for example, `fix: handle empty
browser list`). PR CI runs the Rust and UI checks and builds Fedora RPM and
Ubuntu DEB packages; keep the PR green.

Contributions are licensed under the same GPL-3.0-only terms as Linh
(inbound = outbound). By submitting a contribution, you agree to license it
under those terms.
