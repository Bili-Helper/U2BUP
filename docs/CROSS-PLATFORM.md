# 跨平台构建基础（v0.4）

本批次提供构建矩阵、媒体组件准备脚本、安装包配置和实验性 headless 容器。Windows 已本机构建；macOS、Linux 与 Docker 必须通过各自 CI 和实机验收后才能称为可分发支持。未触发远端构建或发布。

| 目标 | 已配置 | 当前边界 |
| --- | --- | --- |
| Windows x64 | Rust/Tauri 编译、测试、便携包 | 本机验证；签名与安装包验收另行进行 |
| macOS Apple Silicon / Intel | 两个原生 runner、app/dmg 配置 | 尚未运行；FFmpeg 动态依赖、签名、公证待验证 |
| Linux x64 | Ubuntu 24.04 编译、deb/AppImage 配置 | 尚未运行；桌面依赖与 keyring 持久性待验证 |
| Docker Linux amd64 / arm64 | 两个原生 runner、Dockerfile、Compose、冒烟脚本 | 本机未安装 Docker，尚未实际构建容器 |

## CI

`.github/workflows/build.yml` runs on `app/**` changes and manual dispatch. Fixed Rust 1.98.1 / Node 24.21.0: WebUI, tests, Rust fmt/check/test, release binaries, headless smoke. Artifacts are `unbundled-*` (no FFmpeg) and must not be treated as final desktop installers.

`.github/workflows/release.yml` runs **only on `v*` tags**:

| Tag shape | Channel | Package |
| --- | --- | --- |
| Contains `-` (e.g. `v0.7.0-preview.1`) | Preview / Pre-release | **A** — unbundled multi-platform zips |
| Semver only (`v0.7.0`) | Stable Release | **B** — Windows portable+FFmpeg; macOS DMG; Linux AppImage+deb |

Stable media comes from reviewed pins under `app/release/media/` via `scripts/fetch-release-media.mjs` (SHA-256 verified; no floating `latest` without pin updates). macOS DMGs are **unsigned** until Apple notarization secrets are configured. Linux has no DMG; AppImage is the single-file artifact.

Node version from [official archive](https://nodejs.org/en/download/archive/v24.21.0); runners from [GitHub runner-images](https://github.com/actions/runner-images). Cargo.lock and package-lock.json pin app deps. Container base images still need digest locking before calling Docker a supported product.

## 媒体组件与安装包

在目标 OS/CPU 上准备可运行的 FFmpeg 与 FFprobe，执行：

```sh
node app/scripts/stage-media.mjs --source /path/to/ffmpeg --target aarch64-apple-darwin --manifest reviewed-media.json
```

清单格式见 `app/scripts/media-manifest.example.json`。脚本校验目标 OS/CPU、两个二进制的 SHA-256、版本命令及许可证文件，再复制到 `app/desktop/resources`。本地试验可显式传 `--development`，但生成清单会标记为开发用途。脚本不会下载或猜测第三方二进制来源。

准备资源后在 `app/desktop` 执行（本机需有 Rust、Node 与对应桌面系统依赖）：

```sh
../web/node_modules/.bin/tauri build --config tauri.bundle.macos.json
# Linux:
../web/node_modules/.bin/tauri build --config tauri.bundle.linux.json
```

Tauri CLI 的运行目录应为 `app/desktop`；先构建 `app/web`。安装包配置引用现有图标和 `resources/*`。macOS 最低系统暂设 11.0，需要实机确认。正式分发还需处理动态库可携带性、签名、公证与许可证对应源码；清单验证不等于这些步骤已完成。

Windows 继续使用 `app/scripts/build-portable.ps1`，会包含内置媒体工具。

## Docker headless 试验

在 `app` 目录设置 `U2BUP_LIBRARY` 为现有素材目录，运行：

```sh
export U2BUP_LIBRARY=/absolute/path/to/recordings
docker compose up --build -d
docker compose logs u2bup
```

PowerShell 使用 `$env:U2BUP_LIBRARY = 'C:\path\to\recordings'`。日志中的启动链接用于创建会话，不要公开分享。Compose 默认只发布到宿主机回环地址，原片以只读方式挂载；数据库和成品各用独立命名卷，容器以 UID/GID 10001 运行、根文件系统只读。输入目录需允许该用户读取。`U2BUP_PORT` 可改变宿主机端口。

镜像内置发行版 FFmpeg/FFprobe。容器模式目前支持素材扫描、元信息、合并/切割与本地任务；**YouTube 连接和上传明确禁用**，界面显示原因，不读取容器中的系统 keyring。容器 OAuth 与凭据持久化是后续独立工作。

新增参数：`--headless --bind 0.0.0.0 --public-origin http://127.0.0.1:4173`。非回环监听必须同时声明 headless 和浏览器访问 origin。服务严格检查 Host 和 Origin，HTTPS origin 下会话 cookie 带 Secure。单用户令牌仍是访问边界，不构成多用户系统。

CI 运行 `node app/scripts/smoke-container.mjs <image>`，检查镜像启动、映射端口、会话鉴权、跨来源拒绝、只读素材挂载、实际 FFprobe 扫描和 SIGTERM 退出。该脚本尚未在本机执行。

`node app/scripts/smoke-headless.mjs` 可单独检查原生服务的 headless 边界，使用隔离目录与回环端口，已在 Windows 通过 8 项检查；CI 会对各目标原生服务执行。它不验证容器镜像或 Linux SIGTERM 行为。

## 正式发行前的剩余事项

- 在各平台实际执行 CI、GUI 启动、中文路径和媒体处理测试，验证暂停/恢复及退出回收。
- 提供经核实的 FFmpeg 来源、哈希、许可证与对应源码获取说明；macOS/Linux 检查所有动态依赖。
- 完成安装、升级、卸载、签名和公证流程；Docker 固定镜像 digest 并生成可追溯清单。
- 核实根目录 Apache-2.0 LICENSE 与 Cargo 中 GPL-3.0-only 声明的关系；本批次没有擅自变更许可。
