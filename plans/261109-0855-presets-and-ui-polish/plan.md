# Plan — Rule presets and UI polish

Status: in progress · Created: 2026-11-09 · Repo: `tkhieu/linh` · Builds on the merged Fedora MVP.

## Objective

Add one-click rule presets (Google Meet, Microsoft Teams, Slack, Zoom) and polish the UI
(Settings layout, picker look, dark mode, browser icons) on Fedora 44 GNOME.

## Decisions (confirmed by the user, 2026-11-09)

- Meet → a browser/profile the user picks.
- Teams and Zoom → their desktop app when installed, else a fallback browser.
- Slack → a browser/profile (the Slack Linux app only takes `slack://` links and exact
  channel links need a team ID that https links do not carry).
- UI: Settings split into General / Browsers / Rules; picker with icons, larger rows, key
  hints; dark mode follows GNOME (`prefers-color-scheme`); browser icons in Settings and picker.

Evidence: `slack.desktop` declares only `x-scheme-handler/slack`; Teams for Linux
(`com.github.IsmaelMartinez.teams_for_linux`) declares `x-scheme-handler/msteams` and its
upstream docs say it accepts https Teams links; Zoom's Flathub entry declares `zoommtg`.

## Contract (frozen — backend and UI build against it in parallel)

Config (TOML, backward compatible — existing files parse unchanged):

```toml
[[rules]]
name = "Teams"
preset = "teams"                      # optional: "meet" | "teams" | "slack" | "zoom"
match = ["teams.microsoft.com/l/meetup-join/*", "teams.microsoft.com/meet/*",
         "teams.live.com/meet/*", "teams.live.com/l/meetup-join/*"]
open_in = { app = "teams", fallback = { browser = "google-chrome" } }
```

- `OpenIn` = browser `Target` table | `"prompt"` | `AppTarget { app: "teams" | "zoom", fallback: Option<Target> }`.
- `Rule.preset: Option<String>` is display metadata only; the `match` patterns are what route.
- Preset patterns: Meet `meet.google.com`; Teams as above; Slack `*.slack.com/archives/*`,
  `app.slack.com/client/*`; Zoom `zoom.us/j/*`, `*.zoom.us/j/*`.
- App launch: Teams passes the original https URL to the app's Exec; Zoom rewrites
  `https://<host>.zoom.us/j/<id>?pwd=x` → `zoommtg://<host>.zoom.us/join?action=join&confno=<id>&pwd=x`
  (no pwd → omit it). App not installed, URL not convertible, or launch error → `fallback`,
  else Primary. Never a shell.

New commands (names/types exact; mirrored in `ui/src/api.ts`):

- `list_presets() -> Vec<Preset>`; `Preset { id, name, patterns: Vec<String>, app: Option<String>, app_installed: bool }`.
- `get_icon(icon: String) -> Option<String>` → `data:image/png;base64,…` or `data:image/svg+xml;base64,…`;
  absolute path or freedesktop theme lookup (hicolor + Flatpak export icon dirs + `/usr/share/pixmaps`);
  PNG/SVG only, ≤ 512 KiB; never enables the asset protocol.

## Phases

| # | Branch | Owner | Files owned |
|---|---|---|---|
| 1 | `feat/presets-backend` | claude_code | `crates/**`, `src-tauri/**` |
| 2 | `feat/ui-polish` | codex | `ui/**` (incl. `api.ts`) |
| 3 | Integration review + manual check | jacky + reviewer + user | — |

Also folds in issue #9 (phase-label comments → phase 1; weak prompt race test → phase 2).

## Done means

1. CI green on `main`; RPM builds.
2. Settings has General / Browsers / Rules sections; Rules offers "Add preset" for the four presets, prefilled with the patterns above.
3. Teams meeting link opens in Teams for Linux; Zoom `/j/` link becomes `zoommtg://…` for the Zoom app; with the app absent both go to the fallback browser.
4. Meet and Slack presets open in the chosen browser/profile.
5. Existing `linh.toml` files load unchanged; hand-written `open_in = { app = … }` works.
6. Picker shows icons, larger rows and key hints; Settings shows browser icons.
7. UI follows GNOME dark/light.
8. Unit tests cover config compatibility, preset patterns, Zoom rewrite, fallback, icon lookup, and the new UI.

## Out of scope

Slack desktop deep links, other presets, macOS/Windows, Flatpak packaging of Linh.
