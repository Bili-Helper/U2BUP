# U2BUP

<p align="center">
  <strong>Unified livestream archive → local processing → YouTube publish workbench</strong><br/>
  统一录播归档、本地后处理与 YouTube 发布的本机工作台
</p>

<p align="center">
  <a href="https://github.com/Bili-Helper/U2BUP/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/Bili-Helper/U2BUP?style=social" /></a>
  <a href="https://github.com/Bili-Helper/U2BUP/network/members"><img alt="GitHub forks" src="https://img.shields.io/github/forks/Bili-Helper/U2BUP?style=social" /></a>
  <a href="https://github.com/Bili-Helper/U2BUP/issues"><img alt="GitHub issues" src="https://img.shields.io/github/issues/Bili-Helper/U2BUP" /></a>
  <a href="LICENSE"><img alt="License: GPL-3.0" src="https://img.shields.io/badge/license-GPL--3.0-blue.svg" /></a>
  <a href="https://github.com/Bili-Helper/U2BUP/releases"><img alt="Version" src="https://img.shields.io/badge/version-v0.7.0-brightgreen.svg" /></a>
  <a href="https://github.com/Bili-Helper/U2BUP/releases"><img alt="Preview" src="https://img.shields.io/badge/preview-v0.8.0--preview.1-orange.svg" /></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20x64-0078D4.svg" />
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#quick-start">Quick Start</a> ·
  <a href="#roadmap">Roadmap</a> ·
  <a href="#documentation">Docs</a> ·
  <a href="#acknowledgements">Acknowledgements</a> ·
  <a href="#contributing">Contributing</a>
</p>

---

## Background

Live archival workflows usually span several tools: a recorder dumps FLV/MP4 trees, a merger/transcoder prepares YouTube-safe segments, and Studio or ad-hoc scripts rename, tag, and upload. Switching contexts loses room identity, session time, and review history.

**U2BUP** keeps those steps in one local workbench:

- **Rust** service owns media jobs, SQLite state, and YouTube OAuth writes  
- **Vue** UI covers library, pipeline canvas, YouTube pages, and task center  
- **Tauri** desktop shell (and a portable `u2bup-server.exe`) share the same API  

First milestone targets existing **LiveRec** libraries. Real-time multi-platform recording (via biliLive-tools-style adapters) and richer AI assist remain on the roadmap—not claimed as done.

> 中文一句话：把直播录播库、本地合并切割、YouTube 上传与可组合元信息批处理放到同一套本机软件里，批量操作为主，逐步加 AI，不要求用户手写 FFmpeg / 正则。

---

## Features

### Available in **v0.7.0**

| Area | What works today |
| --- | --- |
| **Media library** | Scan LiveRec trees, XML room metadata, filters, multi-select, thumbnails, MP4 preview |
| **Merge & cut** | Continuity planning, duration/size budgets, FFmpeg concat / H.264 cut plans, cancel/retry, artifact validation |
| **YouTube** | Desktop OAuth + PKCE, resumable upload queue, channel sync (ID dedupe), batch snippet/status preview & apply |
| **Navigation** | Grouped sidebar, hash deep links, unified task center |
| **Workflow studio** | Canvas pipelines: parse → identity ledger → classify → title/description → tags → playlists → quality/copyright → thumbnails → AI schema stub |
| **Module plugins (moduleApi 1)** | Builtin nodes as `com.u2bup.builtin.*`; installable **declarative L1** JSON packs; enable/disable; host-owned YouTube intents only |
| **Themes** | Light / dark / system + accent themes |

### Preview in **v0.8.0-preview.1** (multi-library, Phase 1 in progress)

| Area | State |
| --- | --- |
| **Multi-library backend** | SQLite schema v2 (`libraries`, `assets_v2`), library CRUD, `folder` scanner with glob excludes and incremental rescan, paginated `/api/assets`, publish-metadata update API |
| **Welcome & library switching** | First-run welcome screen, `/library/manage` page, add-library dialog (LiveRec / folder / WebDAV / OpenList), sidebar per-library switching |
| **Not yet** | Publish-metadata editor, cross-library flat list, migrating legacy LiveRec assets into `assets_v2`, `--library` auto-creating a LiveRec library |

Design: [`docs/MEDIA-LIBRARY-EXPANSION.md`](docs/MEDIA-LIBRARY-EXPANSION.md).

### Explicitly **not** finished

- Full biliLive-tools live recording inside the package  
- Third-party **L2** sandboxed JS/Wasm module runtime  
- Content ID auto-scan, captions/chapters apply, Analytics-driven SEO claims  
- macOS / Linux / Docker production-ready installs (scaffolding exists; not fully accepted)  
- Public plugin marketplace  

---

## Quick Start

### Windows portable build (recommended locally)

```text
app/dist/U2BUP-<version>-windows-x64/
  U2BUP.exe          # desktop
  u2bup-server.exe   # headless / browser UI
  Start-Web.ps1
  resources/ffmpeg.exe
  resources/ffprobe.exe
```

Double-click `U2BUP.exe`, pick a LiveRec root, then **扫描**.  
Or run `Start-Web.ps1` and open the printed `http://127.0.0.1:…/#token=…` link (token becomes an HttpOnly cookie).

### From source

```powershell
cd app
.\Start-Web.ps1 -Build          # first time: npm + vite + cargo
.\Start-Web.ps1                 # later starts
```

Manual release-style portable pack:

```powershell
cd app
.\scripts\build-portable.ps1
```

Details: [`app/README.md`](app/README.md). License notices for bundled tools: [`app/THIRD-PARTY-NOTICES.md`](app/THIRD-PARTY-NOTICES.md).

### GitHub Releases (tag-triggered)

CI does **not** publish a Release on every commit. Push a version tag:

| Tag | Channel | Artifacts |
| --- | --- | --- |
| `v0.7.0-preview.1`, `v0.7.0-rc.1` | **Preview (A)** — Pre-release | Unbundled `desktop`+`server` zips per OS (**no FFmpeg**) |
| `v0.7.0` | **Stable (B)** — full Release | Windows portable zip **with** FFmpeg; macOS **DMG** (unsigned until notarization); Linux **AppImage** + `.deb` |

Workflow: [`.github/workflows/release.yml`](.github/workflows/release.yml). Media pins: [`app/release/media/`](app/release/media/).

---

## Architecture (short)

```text
Desktop (Tauri) ─┐
Browser WebUI  ──┼─► Vue UI ─► Rust HTTP API ─► SQLite + FFmpeg jobs + YouTube API
                 └─► session auth on localhost
```

Pipeline preview is **pure local**; remote YouTube mutations only run through frozen plans in Rust (`videos.update`, `playlists.*`, `playlistItems.insert`, `thumbnails.set`).

---

## Roadmap

Aligned with [`docs/DEVELOPMENT-ROADMAP.md`](docs/DEVELOPMENT-ROADMAP.md) and [`docs/PRODUCT-ARCHITECTURE.md`](docs/PRODUCT-ARCHITECTURE.md):

| Horizon | Direction |
| --- | --- |
| **Near** | Finish v0.8 multi-library Phase 1 (publish-metadata editor, cross-library list, legacy LiveRec migration); stronger identity ledger UX; L1 module sharing; safer remote batch apply on real channels; upload→playlist/thumbnail chaining |
| **Next** | biliLive-tools adapter for live recording; multi-root libraries; captions/chapters; session/version archive model |
| **Later** | Trusted L2 preview sandbox; optional AI providers; cross-platform signed installs; NAS/Docker headless with proper OAuth |

Priorities may change; unchecked items are **not** release promises.

---

## Documentation

| Doc | Topic |
| --- | --- |
| [`docs/WORKFLOW-MODULE-API.md`](docs/WORKFLOW-MODULE-API.md) | Module capabilities, YouTube API mapping, third-party L1 guide |
| [`docs/WORKFLOW-INTENT-SCHEMA.md`](docs/WORKFLOW-INTENT-SCHEMA.md) | Host Intent kinds |
| [`docs/CHANNEL-WORKFLOW-DESIGN.md`](docs/CHANNEL-WORKFLOW-DESIGN.md) | Channel batch design + implementation status |
| [`docs/BUILD-STATUS.md`](docs/BUILD-STATUS.md) | Per-version verification log |
| [`app/release/media/`](app/release/media/) | Reviewed FFmpeg pins for stable release CI |
| [`docs/CROSS-PLATFORM.md`](docs/CROSS-PLATFORM.md) | Multi-platform CI / packaging boundaries |
| [`docs/DEVELOPMENT-ROADMAP.md`](docs/DEVELOPMENT-ROADMAP.md) | Next-stage plan |
| [`docs/PRODUCT-ARCHITECTURE.md`](docs/PRODUCT-ARCHITECTURE.md) | Original product architecture draft |
| [`app/README.md`](app/README.md) | App-local runbook |

---

## Acknowledgements

U2BUP stands on prior art and infrastructure. Thank you:

| Project / resource | Role |
| --- | --- |
| [biliLive-tools](https://github.com/renmu123/biliLive-tools) | Reference for multi-platform recording UX & APIs (not bundled in v0.7) |
| [FFmpeg](https://ffmpeg.org/) / [Gyan builds](https://www.gyan.dev/ffmpeg/builds/) | Merge, cut, probe, thumbnails |
| [YouTube Data API v3](https://developers.google.com/youtube/v3) | Upload & metadata management |
| [Tauri](https://v2.tauri.app/) | Desktop shell |
| [Vue](https://vuejs.org/) / [Vite](https://vitejs.dev/) / [Lucide](https://lucide.dev/) | Web UI |
| [Axum](https://github.com/tokio-rs/axum) / [Tokio](https://tokio.rs/) / [rusqlite](https://github.com/rusqlite/rusqlite) | Service & storage |
| LiveRec-style archive layouts | First-wave acceptance corpus for local libraries |

Third-party license notes: [`app/THIRD-PARTY-NOTICES.md`](app/THIRD-PARTY-NOTICES.md).

---

## Contributors

Thanks to everyone who has pushed commits and design notes so far (from `git shortlog`):

<table>
  <tr>
    <td align="center"><b>Pili Rec</b></td>
    <td align="center"><b>Huanrui Cao</b></td>
  </tr>
</table>

Want to help? See below—and ★ star / fork the repo if the project is useful.

<p align="center">
  <a href="https://github.com/Bili-Helper/U2BUP/stargazers"><img alt="Star History Chart" src="https://api.star-history.com/svg?repos=Bili-Helper/U2BUP&type=Date" width="600" /></a>
</p>

---

## Contributing

1. Prefer small, reviewable PRs with tests for workflow/preview behavior.  
2. Do **not** commit `.local/`, OAuth secrets, or private channel research dumps.  
3. Pipeline plugins: ship **declarative L1** JSON only unless maintainers accept a new builtin/Intent.  
4. Follow GPL-3.0 for derivative distribution; preserve notices for bundled FFmpeg and dependencies.

Bug reports and feature ideas → [GitHub Issues](https://github.com/Bili-Helper/U2BUP/issues).

---

## License

[GPL-3.0-only](LICENSE) — same family as related recording tooling we reference. Redistributing binaries requires corresponding source and third-party notice compliance.
