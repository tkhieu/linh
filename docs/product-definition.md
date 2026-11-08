# Product Definition — Velja clone (MVP)

Status: đã chốt phạm vi · Ngày: 2026-11-08 · Tên sản phẩm: **Linh**

Repo: `tkhieu/linh` (private) · License: proprietary, all rights reserved

Nguồn tham chiếu: https://sindresorhus.com/velja (đọc trực tiếp ngày 2026-11-08).

## 1. Velja là gì (tóm tắt nghiên cứu)

Velja là "browser picker" cho macOS: app tự đăng ký làm **default browser**, nhận
mọi link được click từ app khác (Slack, Mail, Zoom…), rồi chuyển link đó sang
đúng browser / browser profile / desktop app theo rule — hoặc hiện một **prompt
chọn browser** khi người dùng giữ phím `Fn`.

Tính năng của bản gốc (đầy đủ, để biết mình cắt gì):

| Nhóm | Tính năng Velja |
|---|---|
| Lõi | Làm default browser; "Primary Browser" mở mọi link không khớp rule |
| Rule | Match theo URL (domain/pattern) **hoặc** theo source app (app nơi click link); đích là browser, profile, private window, app native, Shortcut, hoặc "Prompt" |
| Built-in | Google Meet → Chrome; Zoom, Teams, Figma → desktop app |
| Prompt | Hiện danh sách browser; phím số / phím chữ gán riêng; mũi tên + Enter; Cmd-click = tạo rule cho domain; Option = copy/share; Ctrl = mở nền |
| Profile/Private | Chrome/Edge/Brave/Vivaldi/Firefox profile; incognito/private window |
| Nâng cao | Expand short URL, bỏ tracking params, JS transform URL, history/log window, system services, browser extensions (Safari/Chrome/Firefox), Shortcuts integration |
| Khác | Menu bar icon, trial/paid ($10), macOS-only |

## 2. Vấn đề & người dùng

**Vấn đề:** người dùng có nhiều browser/profile (công việc vs cá nhân) nhưng OS
chỉ cho một default browser, nên link luôn mở sai chỗ.

**Người dùng mục tiêu MVP:** dev / knowledge worker dùng ≥2 browser hoặc ≥2
Chrome profile, trên **macOS, Windows hoặc Linux**.

**Khác biệt so với Velja:** cross-platform, config là file text có thể
sync/version.

## 3. Phạm vi MVP

### Must-have (P0)

> **Thứ tự giao hàng:** bản đầu tiên chỉ là Fedora (Linux); macOS và Windows
> làm sau. Các mục Linux bên dưới được điều chỉnh bởi mục 9.

1. **Đăng ký làm default browser** trên cả 3 OS và nhận URL `http`/`https`.
   - macOS: khai báo URL scheme trong `Info.plist`, nhận qua Apple Event.
   - Windows: đăng ký ProgID + `Capabilities` + `RegisteredApplications`; mở
     trang *Settings › Default apps* cho user tự chọn (Windows không cho set bằng code).
   - Linux: file `.desktop` với `x-scheme-handler/http(s)`, set qua `xdg-settings`.
   - Chỉ chạy **một instance**; lần mở thứ hai chuyển URL cho instance đang chạy.
2. **Tự phát hiện browser đã cài** (Chrome, Edge, Brave, Vivaldi, Chromium,
   Firefox, Safari trên mac) và **profile Chromium** (đọc `Local State`).
3. **Primary browser**: link không khớp rule nào mở ở đây.
4. **Rule theo URL**: match domain (kể cả subdomain) hoặc wildcard pattern →
   đích là browser, browser + profile, hoặc private window. Rule duyệt theo thứ
   tự, rule đầu tiên khớp thắng.
5. **Prompt chọn browser**: cửa sổ nhỏ hiện gần con trỏ (trên Linux: giữa màn
   hình), chọn bằng phím `1–9`, mũi tên + `Enter`, `Esc` để huỷ. Ba chế độ:
   *luôn hiện* / *chỉ khi giữ phím modifier* / *không bao giờ*. Modifier mặc
   định: **`Ctrl`** trên Windows, **`Option`** trên macOS (đổi được trong
   Settings). Linux không có chế độ modifier (xem mục 9). Một rule cũng có thể chọn đích là "Prompt".
6. **Tray / menu bar icon**: đổi Primary browser nhanh, mở Settings, Quit.
7. **Settings UI** tối giản: chọn Primary browser, danh sách browser hiện trong
   prompt, CRUD rule, chế độ prompt.
8. **Rule theo source app (chỉ macOS)**: rule có thể match theo app nơi click
   link (lấy từ sender của Apple Event). Trên Windows/Linux, phần tạo rule theo
   source app bị tắt trong UI; rule source app có sẵn trong config bị bỏ qua
   (không lỗi).
9. **Config** lưu file **TOML** (`linh.toml`) ở thư mục config chuẩn của OS
   (user sửa tay được, có comment).

### Nice-to-have (P1 — chỉ làm nếu rẻ)

- Rule theo source app trên **Windows** — chỉ là heuristic (foreground window /
  parent process), có thể sai. Trên Linux source app tắt, không nằm trong MVP.
- Cmd/Ctrl-click trong prompt để tạo rule nhanh cho domain hiện tại.
- Built-in rule mở Zoom/Teams/Figma/Meet bằng app native.
- Firefox profile và Edge InPrivate / Firefox private window đầy đủ.

### Non-goals (không làm trong MVP)

Short URL expansion, bỏ tracking params, JS transform URL, history/log window,
browser extensions, system services, Shortcuts/automation, mailto routing,
trial/license/payment, auto-update, i18n (UI chỉ tiếng Anh).

## 4. Luồng chính

```
User click link trong app X
  → OS gọi Linh (default browser) với URL
  → Linh: modifier đang giữ? (không áp dụng trên Linux) ── có ─→ Prompt
          rule nào khớp (URL, hoặc source app trên macOS)?
                               ── có ─→ đích của rule (có thể là Prompt)
          còn lại             ───────→ Primary browser
  → spawn browser với args phù hợp (profile / private)
```

Lần đầu chạy: onboarding 1 màn hình → "Make Linh the default browser" → chọn Primary browser.

## 5. Kiến trúc đề xuất

- **Tauri v2** (Rust backend + webview UI nhẹ cho Settings và Prompt).
  Plugin: `single-instance`, `deep-link` (nhận URL), tray icon.
- **Crate Rust thuần `linh-core`**: rule matching, model config, build lệnh
  launch browser — không phụ thuộc UI, unit test được trên CI cả 3 OS.
- **Module platform** (`#[cfg(target_os)]`): phát hiện browser, đăng ký default
  handler, đọc trạng thái modifier key (macOS/Windows), phát hiện source app (macOS).
- Launch browser bằng `std::process::Command`: Chromium dùng
  `--profile-directory=<dir>`, `--incognito` (Edge: `--inprivate`); Firefox dùng
  `--private-window`; macOS dùng `open -a <App> --args …` (hoặc `-b <bundle id>`).
- Phân phối: `.dmg` (mac), `.msi`/NSIS (Windows), `.rpm` (Fedora; `.deb`/`.AppImage`/Flatpak sau) qua `tauri build`. Code signing là việc sau MVP.

## 6. Acceptance criteria

Bản Fedora đầu tiên dùng bộ tiêu chí riêng trong [plan](../plans/261108-1654-linh-fedora-mvp/plan.md);
các tiêu chí dưới đây áp dụng khi đủ 3 OS.

1. Trên mỗi OS (macOS, Windows 11, một distro Linux desktop có GNOME hoặc KDE),
   sau onboarding, Linh là default browser và click link trong một app khác
   (vd. terminal `xdg-open` / `start` / `open`, hoặc app chat) đi qua Linh.
2. Link không khớp rule mở trong Primary browser trong < 300 ms tính từ lúc Linh nhận URL (đo trên máy dev).
3. Rule `meet.google.com → Chrome` và rule `*.company.com → Chrome profile "Work"` hoạt động; rule đầu tiên khớp thắng.
4. Giữ modifier khi click (macOS/Windows; trên Linux: chế độ `always` hoặc rule → Prompt) → prompt hiện; chọn được bằng phím số và bằng chuột; `Esc` huỷ không mở gì.
5. Mở link thứ hai khi Linh đang chạy không tạo instance mới.
6. Sửa config bằng tay rồi khởi động lại thì thay đổi có hiệu lực; config hỏng → Linh báo lỗi và vẫn mở URL (không nuốt link). Vì config hỏng thì không đọc được Primary browser, Linh mở bằng browser đầu tiên phát hiện được.
7. Trên macOS, rule "link click trong Slack → Chrome profile Work" hoạt động; trên
   Windows/Linux UI không cho tạo rule source app và rule source app trong config
   bị bỏ qua không lỗi.
8. `linh-core` có unit test cho rule matching và build launch args; CI build xanh trên cả 3 OS.

## 7. Rủi ro đã biết

- **Windows** không cho app tự set default browser — bắt buộc user bấm trong Settings.
- **Modifier `Ctrl`** trên Windows: trong Word/Outlook phải `Ctrl`-click mới mở
  link, nên prompt sẽ luôn hiện ở các app này. Chấp nhận cho MVP; user đổi
  modifier trong Settings nếu vướng.
- **Source app** trên macOS thất bại với Terminal và app gọi lệnh `open` (giống
  Velja); trên Windows/Linux (đặc biệt Wayland) không đáng tin → để sau MVP.
- **Linux Wayland**: vị trí con trỏ và đọc modifier toàn cục bị hạn chế; prompt có thể phải hiện giữa màn hình.
- Browser mở link vào cửa sổ gần nhất (có thể là private/profile khác) — giới hạn giống Velja, ghi vào FAQ.

## 8. Quyết định đã chốt (2026-11-08)

- Tên: **Linh**.
- Modifier bật prompt: **`Ctrl`** trên Windows, **`Option`** trên macOS
  (vì `Ctrl`-click trên mac là chuột phải); cấu hình được. Linux: không có modifier (mục 9).
- Source app: có trong MVP **chỉ cho macOS**; Windows/Linux tắt, làm sau.
- Config: **TOML** — có comment, là định dạng chuẩn của hệ Rust; YAML bị loại vì
  dễ lỗi thụt lề / ép kiểu ngầm và crate `serde_yaml` đã ngừng bảo trì.
- Repo `tkhieu/linh`, private, license proprietary.

## 9. Linux (Fedora) — quyết định riêng (2026-11-08)

Bản đầu tiên chỉ build cho **Fedora 44, GNOME trên Wayland**. macOS và Windows làm sau.

- **Không dùng phím modifier trên Linux.** Wayland không cho app thường đọc trạng
  thái bàn phím toàn hệ thống. Prompt chỉ hiện khi chế độ prompt là `always`
  hoặc khi rule có đích là Prompt. Modifier `Ctrl` vẫn là mặc định cho Windows.
- **Prompt hiện giữa màn hình**, vì Wayland không cho app đặt cửa sổ cạnh con trỏ.
- **Tray icon qua AppIndicator.** GNOME cần extension
  `gnome-shell-extension-appindicator` (`appindicatorsupport@rgcjonas.gmail.com`)
  được bật; KDE hiện tray sẵn.
- **Browser Flatpak** (vd. `org.mozilla.firefox`, `com.google.Chrome`) được phát
  hiện và mở qua `flatpak run`, bên cạnh browser cài bằng RPM.
- **Phân phối:** file `.rpm` build bởi CI trong container `fedora:44`; Flatpak để sau.
- **Source app** vẫn tắt trên Linux như mục 8.
- Kế hoạch triển khai: [plan](../plans/261108-1654-linh-fedora-mvp/plan.md).
