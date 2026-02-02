# reaction-cut-rust

A desktop tool for Bilibili live recording, download, clip/merge/segment, upload, and Baidu Netdisk sync. Built with Tauri + React.

## Features

- Live room subscription and manual/auto recording (with danmaku recording options)
- Multi-part video download
- Clip, merge, and segment workflow for submissions
- Upload/replace/retry for Bilibili submissions
- Optional Baidu Netdisk sync
- Bundled binaries: ffmpeg, ffprobe, aria2c, BaiduPCS-Go

## Requirements

- Node.js 18+
- pnpm 8+
- Rust stable toolchain
- Tauri prerequisites
  - macOS: Xcode Command Line Tools

## Quick Start (dev)

```bash
pnpm install
pnpm tauri dev
```

Frontend only:

```bash
pnpm dev
```

## Build

```bash
pnpm tauri build --bundles dmg
```

Note: DMG creation uses `hdiutil`, which requires a non-sandboxed environment on macOS.

## Bundled binaries

Binaries are expected at `src-tauri/bin/<platform>`.

- macOS: `src-tauri/bin/macos`
- Windows: `src-tauri/bin/windows`
- Linux: `src-tauri/bin/linux`

If you maintain your own binaries, use:

```bash
BIN_SOURCE_DIR="/path/to/bin/macos" pnpm run install-bins
```

## Data location (macOS)

- App data: `~/Library/Application Support/com.tbw.reaction-cut-rust/`
- Logs: `app_debug.log`, `auth_debug.log`, `panic_debug.log`
- Database: `reaction-cut-rust.sqlite3`

## Security & Privacy

See `SECURITY.md` and `PRIVACY.md`.

## License

See `LICENSE` and `THIRD_PARTY_NOTICES.md`.
