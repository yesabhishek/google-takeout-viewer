# Takeout Viewer

A fast, private desktop app for browsing a **Google Photos Takeout** export: hundreds of gigabytes of photos, videos and Live Photos, straight from the folder or external drive you unzipped it to. The interface follows Google Photos: a justified timeline grouped by day, a year scrubber, People, Albums, Favorites, a full-screen viewer and multi-select export.

Built with a Rust core ([Tauri 2](https://tauri.app)) and a small Svelte UI. The app bundle is about 8 MB, and nothing is uploaded, copied or modified.

## Features

- **Timeline like Google Photos.** Justified rows, day headers (small days share a row), a draggable year/month scrubber, pinch or `⌘`-scroll to resize thumbnails, and hover check marks with shift-click range selection.
- **Understands Takeout.**
  - Dates, descriptions, GPS, favorites, archive and trash state, and people come from the JSON sidecars, including every naming variant: `.json`, `.supplemental-metadata.json` and its truncations, `(1)` duplicates, extension-less names, and names cut at 51 characters.
  - Split archives (`takeout-…-001`, `-002`, …) merge into one library, even when a photo and its sidecar ended up in different parts.
  - Album folders (`metadata.json`) become Albums. Named face groups become People.
  - The copies Takeout puts in both `Photos from YYYY` and album folders are de-duplicated.
  - `-edited` versions replace their originals. The viewer can still show the original.
- **Live Photos and motion photos.** iPhone `HEIC`+`MOV` pairs and Pixel/Samsung motion photos (`.MP.jpg` with an embedded video) play on hover, like Google Photos.
- **Everything plays natively on macOS.** HEIC, JPEG, PNG, GIF, WebP, AVIF, TIFF, RAW (via preview), and MP4/MOV video with seeking. AVI/MKV/WMV get thumbnails when `ffmpeg` is installed.
- **Export.** Export any selection, album, person or search result as the originals, or convert to JPEG, PNG, HEIC, AVIF or TIFF (whatever macOS can encode). You can pick a size and quality, include Live Photo clips, and keep EXIF/GPS. File dates are set to the capture time.
- **Search** by file name, description, person, album, camera, year (`2019`) or month (`july`).
- **Works with the drive unplugged.** Thumbnails are cached locally, so you can browse everything. Originals open once the drive is back.

## Download

Get the latest build from [Releases](https://github.com/yesabhishek/google-takeout-viewer/releases):

- **macOS 13+** (Apple Silicon and Intel): the `universal.dmg`.
- **Windows 10/11** (preview): the `x64-setup.exe`. It installs for the current user and needs no admin rights. For iPhone photos, install Microsoft's free [HEIF Image Extensions](https://apps.microsoft.com/detail/9pmmsr1cgpwg) from the Microsoft Store; without it, HEIC files don't preview. iPhone videos (HEVC) also need the [HEVC Video Extensions](https://apps.microsoft.com/detail/9nmzlz57r3t7).

The builds aren't code-signed yet, so the first launch shows a warning:

- **macOS:** open the app once, then go to **System Settings → Privacy & Security** and click **Open Anyway**.
- **Windows:** on the SmartScreen prompt, click **More info → Run anyway**.

## Getting started

Requirements: macOS 13+, [Rust](https://rustup.rs) 1.88+, Node.js 20+. `ffmpeg` (`brew install ffmpeg`) is optional and only needed for AVI/MKV/WMV previews.

```bash
npm install
npm run tauri dev       # run in development
npm run tauri build     # build "Takeout Viewer.app" into src-tauri/target/release/bundle/
```

Then:

1. In Google Takeout, export Google Photos and **unzip every `takeout-*.zip` into one parent folder** (or onto an external drive).
2. Open Takeout Viewer and click **Choose Takeout folder**. You can also drop the folder onto the window. Pick the parent folder, the `Takeout` folder or the `Google Photos` folder; all of them work.
3. Photos appear newest first while the library is indexed. Thumbnails for the whole library are then prepared in the background. Visible photos always come first, and you can pause this in **Library & storage**.

### Keyboard

| Where | Keys |
| --- | --- |
| Grid | `⌘F` search · `⌘A` select all · `Esc` clear selection · `+` / `−` thumbnail size · `⌘`+scroll or pinch to zoom |
| Viewer | `←` `→` previous/next · `i` info · `f` favorite · `+` / `−` or double-click to zoom, drag to pan · `Esc` back |

## How it works

```
src-tauri/src/
  scan.rs      walk → parse sidecars → read file headers (parallel) → SQLite → link
  takeout.rs   sidecar naming rules, album metadata, folder classification (unit-tested)
  meta.rs      fingerprint, size, EXIF (date/GPS/camera), motion-photo detection
  video.rs     minimal MP4/MOV box parser: size, rotation, duration, creation time
  mac.rs       ImageIO (HEIC/RAW/JPEG thumbnails, conversion) + AVFoundation (video frames)
  win.rs       Windows: WIC (HEIC/AVIF/RAW/JPEG decoding via the OS codecs) + Media Foundation (video frames)
  thumbs.rs    thumbnail cache: visible tiles first (LIFO), then background pre-generation
  protocol.rs  gp:// media protocol: thumbnails, originals with byte ranges, Live clips
  export.rs    copy / convert / resize, Live clips, file dates
  api.rs       command surface shared by Tauri IPC and the HTTP dev bridge
src/           Svelte 5 UI. The grid is virtualized: ~40 DOM tiles regardless of library size
```

- **Indexing only reads file headers** (first 128 KB) plus the tiny JSON sidecars, so it scales with the number of files, not their size. Rescans skip unchanged files.
- **Thumbnails** are 512 px JPEGs decoded with macOS hardware decoders (ImageIO / AVFoundation) and keyed by content fingerprint. Expect roughly 30–60 KB each, so about 5–8 GB for 150k photos.
- **The timeline** is sent to the UI as one compact binary array (23 bytes per item) and laid out in JS.

Measured on an Apple Silicon Mac with a synthetic 60,000-photo Takeout (3 split archives, 30 albums, 20 people) on the internal SSD:

| | |
| --- | --- |
| First index (62,100 files) | 3.3 s |
| Rescan, nothing changed | 2.6 s |
| Timeline payload / decode / layout | 1.4 MB / 11 ms / 83 ms |
| Thumbnails (small test images) | 60,000 in 21 s |

Real libraries on an external USB hard drive are I/O-bound and will be slower, but this is a one-time cost. After that, browsing comes from the local index and thumbnail cache.

## Where things are stored

`~/Library/Application Support/com.yesabhishek.takeoutviewer/` holds `library.sqlite3` and `thumbs-v1/`. Your Takeout folder is only ever read. Removing a folder in **Library & storage** forgets its index without touching the files.

## Command line

The app binary also runs headless, which is handy for indexing a huge drive overnight or for UI development:

```bash
APP="src-tauri/target/release/takeout-viewer"
$APP index "/Volumes/Photos/Takeout export" --thumbs   # index + pre-generate thumbnails
$APP serve --port 1421                                  # HTTP bridge for `npm run dev` in a browser
```

`--data <dir>` (or `TAKEOUT_VIEWER_DATA=<dir>` for the GUI) uses a separate library folder.

## Developing the UI in a browser

```bash
cargo build --manifest-path src-tauri/Cargo.toml
src-tauri/target/debug/takeout-viewer serve --data /tmp/tv-data --source "/path/to/Takeout"
npm run dev   # open http://localhost:1420 — it talks to the bridge on 127.0.0.1:1421
```

## Releasing

[`.github/workflows/release.yml`](.github/workflows/release.yml) builds a universal macOS `.dmg` and a Windows NSIS installer.

1. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. Commit, then tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. When both jobs finish, review the draft on the Releases page and publish it.

To build without releasing (for example, to try the Windows build), run the **Release** workflow by hand from the Actions tab. The installers are uploaded as workflow artifacts.

## Notes

- macOS is the main platform. Windows builds decode images with WIC and video frames with Media Foundation, falling back to the `image` crate and `ffmpeg`. This has had very little testing. HEIC/HEIF works on Windows when the HEIF Image Extensions are installed, and AVIF needs the AV1 Video Extension. Export can convert to JPEG, PNG, TIFF or WebP there. Linux uses the portable fallback only.
- The original single-file browser viewer lives in [`legacy-web/`](legacy-web/).

## License

MIT
