# Plan — Linh Fedora MVP

Status: in progress · Created: 2026-11-08 · Repo: `tkhieu/linh` (private)
Source: [product definition](../../docs/product-definition.md) · Brief confirmed via `/ak-atoz` gate 1.

## Objective

Ship Linh (Velja-style browser picker, Tauri v2 + Rust) as an installable RPM
for Fedora 44 GNOME/Wayland.

## Done means

1. Private repo `tkhieu/linh`; Linux CI (fedora:44 container) runs fmt, clippy, tests, build — green on `main`.
2. Build produces `.rpm`; `sudo dnf install ./linh-*.rpm` works; Linh appears in app grid.
3. After onboarding, `xdg-settings get default-web-browser` returns `linh.desktop`.
4. `xdg-open <url>` with no matching rule opens Primary browser; rules `meet.google.com → Chrome` and `*.company.com → Chrome profile` work; first match wins; detects RPM Chrome and Flatpak Chrome/Firefox.
5. "Private" destination opens Chrome incognito / Firefox private window.
6. Prompt mode "always" and rule→Prompt show picker; 1–9, arrows+Enter, mouse; Esc opens nothing.
7. Second `xdg-open` while running spawns no new Linh process.
8. AppIndicator tray icon: change Primary browser, open Settings, Quit.
9. Hand-edited `~/.config/linh/linh.toml` applies after restart; broken config → error shown, URL still opens in first detected browser.
10. Linux: UI cannot create source-app rules; source-app rules in config ignored without error.
11. `linh-core` unit tests (rule matching, launch args) pass; receive→spawn < 300 ms (logged).
12. `docs/product-definition.md` records Linux-specific decisions.

## Out of scope

macOS/Windows builds, Ctrl modifier on Linux, Linux source-app, Firefox profiles,
Flatpak packaging, code signing, auto-update, product-definition non-goals.

## Phases

| # | Branch | Owner | Files owned | Depends on |
|---|---|---|---|---|
| 1 | `feat/scaffold-core` | claude_code | workspace root, `crates/linh-core/**`, `src-tauri/**` skeleton + `commands.rs` contract, `ui/**` skeleton, `.github/**` | — |
| 2 | `feat/linux-backend` | claude_code | `src-tauri/**` | 1 merged |
| 3 | `feat/ui` | codex | `ui/**` | 1 merged (parallel with 2) |
| 4 | `docs/linux-decisions` | jacky (docs) | `docs/**` | — |
| 5 | Integration verification + final isolated review | jacky + reviewer | — | 2, 3 merged |

Phase 1 freezes the Tauri command contract (names, argument and return types)
so phases 2 and 3 can run in parallel without touching each other's files.

## Defaults (reversible)

React + Vite + TypeScript with pnpm; CI in `fedora:44`; Chrome profile
"regulator.ca" stands in for "Work" in manual tests; max 2 concurrent build agents
(~5 GB free RAM); broken-config fallback = first detected browser; prompt
window centered (Wayland cannot position windows).

## Risks

- Local Tauri system deps need `sudo dnf install` (user); until then local
  verification of the Tauri crate relies on CI.
- GUI items (6, 8, onboarding) need a short manual check by the user.
- Non-trivial PRs wait for the user to merge (jacky auto-merge rule).
