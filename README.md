# linkshot

An unofficial, Lightshot-style screenshot tool for **Ubuntu/Linux** (and macOS).
Launch it → the whole screen freezes under a dimmed overlay → drag to select a
region → a **floating toolbar** appears next to your selection (pen / line / arrow /
rect, colors, upload / save / copy) → annotate in place → the public link is copied
to your clipboard. This is the flameshot/Lightshot interaction model, not a big
window.

It is the cross-platform answer to "I want Lightshot on Linux". It does **not** use
Lightshot's `prnt.sc` service (see *Why not prnt.sc?* below); it uploads through the
**official Imgur API** instead, which offers documented anonymous uploads.

## Why not prnt.sc?

Reverse-engineering the original Lightshot Windows app (see
[`docs/REVERSE_ENGINEERING.md`](docs/REVERSE_ENGINEERING.md))
showed that every upload to `upload.prntscr.com` must carry an `app_token`:

```
app_token = encode( AES-128-CBC( key=<16B embedded>, iv=<16B embedded>, plaintext ) )
```

That token is a deliberate **client-authentication signature** Skillbrains uses to
ensure requests come from their official app. Forging it to use their servers as an
unauthorized client is a Terms-of-Service / authorization question (and is fragile —
the `app_id` can be revoked and the scheme changed at any time). So linkshot keeps the
exact Lightshot **UX** but targets a service with a real public API.

## Build

Requires a Rust toolchain (`rustup`, stable).

```bash
cargo build --release
# binary at target/release/linkshot
```

Install system-wide:

```bash
sudo install -m755 target/release/linkshot /usr/local/bin/linkshot
install -Dm644 assets/linkshot.desktop ~/.local/share/applications/linkshot.desktop
```

## Runtime dependencies (Linux capture)

linkshot grabs the **whole screen** with whatever screenshot tool you already have,
then does selection/annotation inside its own overlay. Install **one** of these:

| Desktop / session | Package |
|---|---|
| Sway / Hyprland (wlroots Wayland) | `grim` |
| GNOME (Wayland or X11) | `gnome-screenshot` |
| KDE Plasma | `spectacle` |
| Any X11 | `maim`, or `scrot`, or ImageMagick (`import`) |

Example on Ubuntu GNOME: `sudo apt install gnome-screenshot`.

## Imgur Client-ID (one-time)

Anonymous Imgur uploads need a free Client-ID:

1. Go to <https://api.imgur.com/oauth2/addclient>
2. Choose **"OAuth 2 authorization without a callback URL"** (anonymous usage).
3. Copy the **Client ID**.

Provide it either way:

- Environment variable: `export IMGUR_CLIENT_ID=xxxxxxxxxxxxxxx`
- Or paste it into the field shown in the app (saved to
  `~/.config/linkshot/config.json`).

## Usage

1. Run `linkshot` (ideally bound to a hotkey — see below). The screen freezes under
   a dimmed overlay.
2. **Drag** to select the region you want. Outside the selection stays dimmed.
3. A floating toolbar appears next to the selection. Pick a tool (pen / line / arrow
   / rect), a color and width, then drag inside the selection to annotate.
   **Ctrl+Z** undoes the last shape; the crop button re-selects.
4. **Upload** (cloud icon) → the Imgur link is copied to your clipboard. Or **Save**
   (disk icon) to write a PNG to your Pictures folder. **Esc** closes the overlay.

## Bind a global hotkey (the Lightshot way)

linkshot is meant to be launched by a hotkey — running it *is* the capture, just like
pressing PrintScreen in Lightshot. It does not grab the hotkey itself (unreliable on
Wayland); bind one in your desktop settings:

**GNOME / Ubuntu:** Settings → Keyboard → *View and Customize Shortcuts* → *Custom
Shortcuts* → **+**:
- Name: `linkshot`
- Command: `/usr/local/bin/linkshot`
- Shortcut: e.g. `Print` (clear the default PrintScreen binding first if needed).

## What works / roadmap

- ✅ Full-screen grab → in-overlay region selection (wlroots / GNOME / KDE / X11 / macOS)
- ✅ Floating toolbar next to the selection; icons via Phosphor font
- ✅ Annotate: pen, line, arrow, rectangle; color + width; undo; reselect
- ✅ Upload to Imgur (anonymous), clipboard copy, save to file
- 🔜 Text tool, highlighter, blur/pixelate
- 🔜 **System tray icon** (resident background app) — Linux StatusNotifierItem via
  `ksni`; deferred because it can't be verified on the macOS dev box
- 🔜 Configurable backends (0x0.st, S3/SFTP), `.deb` / AppImage packaging
