# linkshot — Implementation Plan (next features)

Kế hoạch triển khai 4 tính năng. Mỗi mục: **mục tiêu → crate → file đụng tới →
các bước → phác code → cạm bẫy → cách test**. Tất cả bám theo code hiện có:
`App` (main.rs), `Mode {Selecting, Editing}`, `editor::{Tool, Shape, Annot}`,
`editor::render_png`, `capture::capture_fullscreen`.

---

## (a) System tray icon (resident background app)

**Mục tiêu:** chạy nền có icon trên statusbar; menu *Capture* / *Quit*. Bấm
Capture → mở overlay chụp như hiện tại.

**Quyết định kiến trúc — KHÔNG nhúng tray vào eframe event loop.**
eframe chiếm main thread. Tách hẳn 2 vai trò trong cùng 1 binary, phân biệt bằng cờ:

- `linkshot`            → chế độ chụp hiện tại (capture → overlay → exit).
- `linkshot --tray`     → daemon nền chạy `ksni`; mỗi lần bấm *Capture* thì
  `spawn` lại chính nó **không cờ** (= 1 lần chụp). Né hoàn toàn việc ghép tray
  với winit/eframe → ít rủi ro nhất.

**Crate:** `ksni = "0.2"` (Linux-only — bọc `#[cfg(target_os = "linux")]`).

**File:** `Cargo.toml`, `src/main.rs` (parse cờ), `src/tray.rs` (mới),
`assets/linkshot-tray.desktop` (autostart), `README.md`.

**Các bước:**
1. Đầu `main()` parse args trước khi capture:
   ```rust
   #[cfg(target_os = "linux")]
   if std::env::args().any(|a| a == "--tray") {
       return tray::run(); // blocking, không trả về cho tới khi Quit
   }
   ```
2. `src/tray.rs`:
   ```rust
   use ksni::{Tray, TrayService, MenuItem, menu::StandardItem};

   struct LinkshotTray;
   impl Tray for LinkshotTray {
       fn icon_name(&self) -> String { "applets-screenshooter".into() }
       fn title(&self) -> String { "linkshot".into() }
       fn menu(&self) -> Vec<MenuItem<Self>> {
           vec![
               StandardItem {
                   label: "Capture".into(),
                   activate: Box::new(|_| {
                       if let Ok(exe) = std::env::current_exe() {
                           let _ = std::process::Command::new(exe).spawn();
                       }
                   }),
                   ..Default::default()
               }.into(),
               MenuItem::Separator,
               StandardItem {
                   label: "Quit".into(),
                   activate: Box::new(|_| std::process::exit(0)),
                   ..Default::default()
               }.into(),
           ]
       }
   }

   pub fn run() -> eframe::Result<()> {
       TrayService::new(LinkshotTray).run().unwrap();
       Ok(())
   }
   ```
3. Autostart desktop entry (`~/.config/autostart/linkshot-tray.desktop`) chạy
   `linkshot --tray` khi đăng nhập.

**Cạm bẫy:**
- GNOME mặc định **không** hiện StatusNotifierItem → cần extension
  *AppIndicator and KStatusNotifierItem Support* (Ubuntu ship sẵn, GNOME thuần phải cài).
- `ksni` cần D-Bus session bus đang chạy (luôn có trong phiên desktop bình thường).
- Spawn process con: con tự chụp full-screen rồi mở overlay — daemon không bị đụng.

**Test:** chỉ trên Ubuntu. `linkshot --tray`, kiểm tra icon xuất hiện, bấm Capture
ra overlay, Quit thoát daemon.

---

## (b) Text tool + Highlighter + Blur/Pixelate

**Mục tiêu:** thêm 3 công cụ kiểu Lightshot.

**Crate (cho xuất text ra PNG):** `ab_glyph = "0.2"` + `imageproc = "0.25"`
(`render_png` dùng `draw_text_mut`). Cần **bundle 1 font TTF**, vd
`assets/DejaVuSans.ttf`, nhúng bằng `include_bytes!`.

**File:** `src/editor.rs` (thêm `Shape`, `draw_egui`, `render_png`),
`src/main.rs` (thêm `Tool`, nút toolbar, xử lý nhập text), `Cargo.toml`, `assets/`.

### Mở rộng model (editor.rs)
```rust
pub enum Tool { Pen, Line, Arrow, Rect, Text, Highlight, Blur }

pub enum Shape {
    Pen(Vec<[f32;2]>),
    Line([f32;2],[f32;2]),
    Arrow([f32;2],[f32;2]),
    Rect([f32;2],[f32;2]),
    Highlight([f32;2],[f32;2]),          // rect tô bán trong suốt
    Blur([f32;2],[f32;2]),               // vùng làm mờ
    Text { pos:[f32;2], text:String, size:f32 },
}
```

### Highlight
- **Preview (`draw_egui`)**: `painter.rect_filled(rect, 0, color_alpha)` với alpha ~90.
- **Export (`render_png`)**: tiny-skia `paint.set_color_rgba8(r,g,b,90)` +
  `paint.blend_mode = BlendMode::Multiply` rồi `pixmap.fill_rect(rect, &paint, ..)`.

### Blur / Pixelate (hiệu ứng theo vùng — KHÔNG phải nét vẽ)
- **Preview**: vẽ 1 rect xám mờ làm placeholder (không blur realtime cho rẻ).
- **Export**: trong `render_png`, **xử lý các Blur trước khi vẽ nét** lên ảnh đã crop:
  ```rust
  // r = vùng (đã ở toạ độ pixel của ảnh crop), clamp trong ảnh
  let sub = image::imageops::crop_imm(&out, x, y, w, h).to_image();
  let blurred = image::imageops::blur(&sub, 8.0);      // hoặc pixelate:
  // let small = imageops::resize(&sub, (w/12).max(1), (h/12).max(1), Nearest);
  // let blurred = imageops::resize(&small, w, h, Nearest);
  image::imageops::replace(&mut out, &blurred, x as i64, y as i64);
  ```
  (Lưu ý: `render_png` đang ghi ra `out: RgbaImage` cuối — chèn bước blur vào đó.)

### Text
- **Nhập liệu (main.rs)**: thêm state `editing_text: Option<usize>` (index annot
  đang gõ). Khi chọn Tool::Text và click trong vùng → tạo `Annot` rỗng, set
  `editing_text`. Mỗi frame đọc input:
  ```rust
  ctx.input(|i| for e in &i.events {
      match e {
          egui::Event::Text(t) => push_to_active_text(t),
          egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => pop_char(),
          egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => commit_text(),
          _ => {}
      }
  });
  ```
- **Preview**: `painter.text(pos, Align2::LEFT_TOP, &text, FontId::proportional(size), color)`.
- **Export**: `imageproc::drawing::draw_text_mut(&mut out, Rgba(color), x, y, scale, &font, &text)`
  với `font = ab_glyph::FontRef::try_from_slice(include_bytes!("../assets/DejaVuSans.ttf"))`.

**Cạm bẫy:**
- Toạ độ text/blur cũng phải đi qua `map_point` + scale như các shape khác trong
  `App::export_png` (nhớ thêm nhánh match cho 3 biến thể mới — nếu thiếu sẽ không compile).
- Cỡ font khi export = `size * scale_w` (giống `width * scale_w` đang làm).
- Blur dùng `sigma` lớn sẽ chậm với vùng to → cân nhắc pixelate nếu cần nhanh.

**Test:** thêm unit test trong `editor.rs` như `render_png_roundtrips_dimensions`:
dựng ảnh có Text + Highlight + Blur, assert decode lại đúng size & pixel có đổi.

---

## (c) Đóng gói `.deb` và AppImage

### `.deb` (đơn giản nhất) — `cargo-deb`
**File:** `Cargo.toml` (thêm metadata), `assets/linkshot.png` (icon 256×256 — cần tạo).
```toml
[package.metadata.deb]
maintainer = "Nguyen Quang Minh <minhnq1@talent.apero.vn>"
section = "graphics"
priority = "optional"
depends = "libc6"
recommends = "gnome-screenshot | grim | scrot | maim | imagemagick"
extended-description = "Lightshot-style screenshot, annotate and upload tool."
assets = [
    ["target/release/linkshot", "usr/bin/", "755"],
    ["assets/linkshot.desktop", "usr/share/applications/", "644"],
    ["assets/linkshot.png", "usr/share/icons/hicolor/256x256/apps/linkshot.png", "644"],
    ["README.md", "usr/share/doc/linkshot/README.md", "644"],
]
```
Build:
```bash
cargo install cargo-deb
cargo deb            # ra target/debian/linkshot_0.1.0_amd64.deb
```
(Sửa `Icon=applets-screenshooter` → `Icon=linkshot` trong `.desktop` nếu dùng icon riêng.)

### AppImage — `linuxdeploy`
**File:** `packaging/build-appimage.sh` (mới).
```bash
#!/usr/bin/env bash
set -euo pipefail
cargo build --release
APPDIR=AppDir; rm -rf "$APPDIR"
install -Dm755 target/release/linkshot       "$APPDIR/usr/bin/linkshot"
install -Dm644 assets/linkshot.desktop        "$APPDIR/usr/share/applications/linkshot.desktop"
install -Dm644 assets/linkshot.png            "$APPDIR/usr/share/icons/hicolor/256x256/apps/linkshot.png"
# cần linuxdeploy + linuxdeploy-plugin-gtk (tải AppImage của linuxdeploy về trước)
./linuxdeploy-x86_64.AppImage --appdir "$APPDIR" --output appimage \
    -d "$APPDIR/usr/share/applications/linkshot.desktop"
```

**Cạm bẫy:**
- Cần **icon thật** (`assets/linkshot.png`) cho cả 2 cách đóng gói.
- AppImage gom lib OpenGL/Wayland/X11 — build trên Ubuntu cũ nhất bạn định hỗ trợ
  để tương thích glibc ngược.
- `.deb` không nhúng tool chụp → để ở `recommends` (không ép cài).

**Test:** `sudo apt install ./linkshot_*.deb` rồi chạy `linkshot`; với AppImage thì
`chmod +x linkshot*.AppImage && ./linkshot*.AppImage`.

---

## (d) GitHub Actions CI

**Mục tiêu:** mỗi push/PR tự `fmt` + `clippy` + `test` + `build --release` trên Ubuntu.

**File:** `.github/workflows/ci.yml` (mới).
```yaml
name: ci
on:
  push: { branches: [main] }
  pull_request:
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - uses: Swatinem/rust-cache@v2
      - name: System deps (eframe/winit/arboard)
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libxkbcommon-dev libwayland-dev \
            libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
            libgl1-mesa-dev
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test --all
      - run: cargo build --release
```

**Tuỳ chọn — release tự động** (`.github/workflows/release.yml`, trigger trên tag
`v*`): build `.deb` (`cargo deb`) + AppImage rồi `softprops/action-gh-release@v2`
đính kèm artifact.

**Cạm bẫy:**
- `clippy -D warnings` sẽ fail nếu còn warning → dọn sạch trước (hiện đang 0 warning).
- Thiếu `libxkbcommon-dev`/`libwayland-dev` là lỗi build điển hình của egui trên CI.
- Bật `Swatinem/rust-cache` để khỏi build eframe lại từ đầu mỗi lần (~vài phút).

---

## Thứ tự đề xuất làm tối nay
1. **(d) CI** trước (5 phút, bắt lỗi cho các bước sau).
2. **(b) Text/Highlight/Blur** (giá trị người dùng cao nhất).
3. **(c) Đóng gói** (cần icon `assets/linkshot.png`).
4. **(a) Tray** (Linux-only, test trên máy Ubuntu).
