# Google Photos Takeout Viewer (Local, Private, Lightweight)

A zero-backend, privacy-first web app to browse a Google Photos Takeout export directly on a computer. It reads media and JSON sidecars locally in the browser, builds a fast, filterable gallery, and opens a clean lightbox—without uploading anything anywhere.

Motto: Own your memories. Browse them locally. No cloud required.

## Why this exists

- Google Takeout gives control of photo data, but viewing it is clunky.
- Existing tools often require a server, an import step, or re-uploading.
- This project keeps things simple: a single HTML file that runs in the browser, understands Takeout’s structure, and respects privacy.

## Highlights

- 100% local. No upload, no server required.
- Works with Google Takeout structure: albums, “Photos from YYYY”, and JSON sidecars.
- Images and videos: lazy-loaded grid, fast object-URL previews.
- Search and filters: filename, caption (from sidecar), type, folder.
- Sorting: newest/oldest, name A→Z/Z→A.
- Lightbox: keyboard navigation, EXIF-time display, open-original.
- Keyboard UX: arrows to navigate, Space/Enter to open focused item, Esc to close.
- Cross-platform: macOS, Windows, Linux. Best with Chromium-based browsers.

## Quick start

**double-click the file**
1) Download index.html from this repo.
2) Open it in Chrome/Edge/Brave.
3) Click “Open folder” (if supported) or “Select directory (fallback)” and choose the “Google Photos” folder inside Takeout.
4) Browse.


Note on browsers:
- Chrome/Edge/Brave: both folder picker and fallback work when served from localhost/https.
- Safari: use the fallback button or drag-and-drop; File System Access API is not supported.
- Firefox: use fallback or drag-and-drop. Firefox doesn’t support a true recursive directory picker.

## How it works

- Directory access: uses the File System Access API when available; otherwise uses input type="file" with directory selection and drag-and-drop.
- Metadata: for each media file, the app looks for sidecar file.ext.json to read timestamps, description, people, and location. If missing, falls back to file modification time.
- Rendering: creates object URLs to display files; nothing is uploaded or copied.
- Performance: lazy rendering and pagination-by-intersection keep memory low.

## Google Takeout layout the app understands

- Takeout/Google Photos/
  - Album A/
    - img1.jpg
    - img1.jpg.json
  - Album B/
    - img2.heic
    - img2.heic.json
  - Photos from 2024/
    - 2024-08-04/
      - img3.png
      - img3.png.json
  - video.mp4
  - video.mp4.json

Duplicates across album folders are common in Takeout exports; the app treats each file path as an item.

## Features in detail

- Filters:
  - Type: images, videos, or all.
  - Folder selector: quickly narrow to a specific album/date folder.
  - Text search: filename or caption (description) from JSON sidecars.
- Sorting:
  - Taken time desc/asc (photoTakenTime/creationTime from JSON; fallback to file time).
  - Name A→Z, Z→A.

## Privacy

- No network uploads. The app reads files locally and creates temporary object URLs.
- Works entirely offline. Try with Wi‑Fi turned off.
- No analytics, no tracking, no external resources by default.

## Limits and tips

- Large libraries (hundreds of thousands of items): initial scanning can take time. Consider browsing subfolders incrementally or using a Chromium browser for faster traversal.
- HEIC: depends on browser support. Chromium-based browsers increasingly support HEIC/AVIF; otherwise, images might not render. Pre-conversion or a server-side thumbnailer is out of scope for this zero-backend app.
- Video thumbnails: the app shows videos as playable elements; it does not generate poster frames.
- Firefox: recursive directory selection is limited; prefer Chrome/Edge for the best UX.

## Roadmap

- Date range filter.
- People and location chips with filtering when present in sidecars.
- Virtualized grid for ultra-large collections.
- Optional PWA install.
- Optional Tauri/Electron wrapper for a native open-folder dialog on all browsers and platforms.

## Development

This project is intentionally buildless. It’s a single HTML file with vanilla JS and CSS.

- Edit index.html and open it locally or via a local server.
- Contributions should keep the zero-backend, zero-build philosophy unless optional advanced paths are clearly separated.

## Contributing

- Fork the repo, create a feature branch, and open a pull request.
- Keep UI fast, accessible, and minimal:
  - Test keyboard navigation.
  - Ensure light/dark contrast meets accessibility guidelines.
  - Avoid heavy dependencies and large bundles.
- For new features, prefer progressive enhancement with graceful fallbacks.

## Community and support

- Issues: Use GitHub Issues for bugs and feature requests. Include browser version, OS, and steps to reproduce.
- Discussions: Share workflows, performance tips, and Takeout edge cases.

## License

MIT. Do whatever helps you own and access your memories—commercial, personal, or educational use welcome. Attribution appreciated but not required.

## Acknowledgments

- Everyone who preserves open, portable formats.
- Browser teams implementing local-first capabilities.
- The open-source community improving privacy-respecting tooling.

Own your memories. Browse them locally. No cloud required.