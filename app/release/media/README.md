# Reviewed FFmpeg pins for stable (B) releases

Each Rust target triple has a JSON pin. Stable CI downloads these archives, verifies SHA-256, stages binaries via `app/scripts/stage-media.mjs`, then builds installers.

| File | Target |
| --- | --- |
| `x86_64-pc-windows-msvc.json` | Windows x64 (Gyan full build) |
| `x86_64-unknown-linux-gnu.json` | Linux x64 static (John Van Sickle) |
| `x86_64-apple-darwin.json` | macOS Intel (evermeet.cx) |
| `aarch64-apple-darwin.json` | macOS Apple Silicon (osxexperts.net) |

Update pins when bumping FFmpeg: change URLs, recompute archive and binary hashes, commit. Do not use floating `latest` URLs without pinning hashes.

Schema fields: `target`, `status` (`ready` \| `pending`), `archives[]` (`url`, `sha256`, optional `format`), `binaries.ffmpeg|ffprobe` (`path` relative to extract root, `sha256`), `licenses` (basenames found in extract or copied from `licenses/`), optional `notes`.
