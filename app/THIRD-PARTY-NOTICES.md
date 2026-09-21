# Third-party components

This is a local development prototype. The public redistribution review is not complete.

- Application source declares GPL-3.0-only. The workspace uses Tauri, Vue, Axum, Tokio, rusqlite/SQLite and other dependencies pinned in Cargo.lock and web/package-lock.json; preserve their individual notices for a public distribution.
- FFmpeg / FFprobe are copied from the user's existing Gyan FFmpeg 8.0 full-build installation. Exact binary hashes and version strings are recorded in resources/media-manifest.json. The original license text is copied alongside them. Project: https://ffmpeg.org/ ; build supplier: https://www.gyan.dev/ffmpeg/builds/ . Binary packaging does not, by itself, complete any required corresponding-source distribution.
- biliLive-tools (https://github.com/renmu123/biliLive-tools) was inspected for compatibility and requirements. Its code is not bundled or executed by this version. Planned future integration must preserve its GPL-3.0 notices and appropriate source materials.
- Application icons use a small original SVG layered mark. UI symbols use the Lucide Vue package under its own license.

Before publishing installers beyond this local workspace, collect and verify complete license notices and corresponding sources for the exact bundled media build and all redistributed libraries.
