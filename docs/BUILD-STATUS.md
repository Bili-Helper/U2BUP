# v0.8.0-preview.1：多素材库基础架构 Phase 1（预览）

日期：2026-09-23。基线：v0.7.0。版本号统一为 `0.8.0-preview.1`，按 release 规则推送该 tag 只生成 GitHub **Pre-release**（各平台 unbundled zip，不含 FFmpeg）。状态：**后端完成；前端欢迎界面与侧边栏多库切换已完成；发布元数据面板、跨库列表与旧数据迁移未完成**，Phase 1 全部完成后再发正式 `v0.8.0`。

版本编号说明：[开发规划](DEVELOPMENT-ROADMAP.md)第 6 节原把 v0.8 定为「工作流与弹幕」。2026-09-23 起以 [MEDIA-LIBRARY-EXPANSION](MEDIA-LIBRARY-EXPANSION.md) 第 9 节为准：v0.8 = 多素材库基础，v0.9 = LLM 集成与发布平台扩展。

## 前端改动（fefea93）

- 新组件 `LibraryManager.vue`：无素材库时显示欢迎界面（3 个快速入口）；`/library/manage` 素材库管理列表；添加素材库对话框（LiveRec / 文件夹 / WebDAV / OpenList，后两者只读提示）。
- `App.vue`：多库状态（`libraries`、`activeLibraryId`）；启动时加载素材库；侧边栏按库列出，显示扫描中/错误状态点，提供「+ 添加素材库」；没有素材库时保留原 LiveRec 入口。
- `navigation.ts` 新增 `/library/folders`、`/library/unpublished`、`/library/manage`；`types.ts` 新增 `LibraryRoot`、`AssetV2` 等类型。

## 工程修正

- Release workflow 的 tag 校验收紧（09009c5）：只接受 `vX.Y.Z`、`vX.Y.Z-preview.N`、`vX.Y.Z-rc.N`，其他 tag 直接报错，不再误发版。
- `build-portable.ps1` 改为读取 `CARGO_TARGET_DIR`，版本号正则支持预发布后缀。此前在设置了该变量的环境（Cursor 沙箱）里，脚本从 `app/target/release` 复制到的是旧 exe：**本地 `dist/` 下的 v0.6.0 / v0.7.0 便携包实际装的是 v0.5.0 程序**。GitHub Actions 发布的 v0.7.0 不受影响。
- 对新增 Rust 代码执行 `cargo fmt`，避免 Build foundation 的格式检查失败。

## 后端改动（df48dd6）

### 数据库 schema 迁移（v1 → v2）

`db.rs` 完全重写：

- 新增 `libraries(id, json)` 表，存放多素材库配置
- 新增 `assets_v2` 表（7 个索引列 + `json` 正文），替代旧的内存 `Library.assets` Vec
- `documents` 表（v1 通用 KV 存储）原封不动保留，全部旧数据安全保留
- 迁移在首次启动时自动执行，`PRAGMA user_version` 由 1 → 2

### 数据模型扩展（model.rs）

- 新增 `LibraryKind` 枚举：`liverec` / `folder` / `webdav` / `openlist`
- 新增 `LibraryRoot` 结构：名称、类型、路径、时区、只读、启用状态、排除规则
- 新增 `AssetV2` 结构：完整发布元数据（pub_title/pub_tags/pub_description 等，对标 YouTube），custom_tags，upload_targets，file_status
- 新增 `fast_file_hash()`：前 1MB + 文件大小 SHA-256[:16]，用于文件移动后 relink

### 多素材库 API（library.rs，全新，591 行 + 569 行测试）

后端路由：

| 方法 | 路径 | 功能 |
|---|---|---|
| GET | `/api/libraries` | 列出所有素材库 |
| POST | `/api/libraries` | 添加素材库 |
| GET/PUT/DELETE | `/api/libraries/{id}` | 详情 / 更新 / 删除 |
| POST | `/api/libraries/{id}/scan` | 触发后台扫描 |
| GET | `/api/libraries/{id}/status` | 扫描进度 |
| GET | `/api/assets` | 跨库分页查询 |
| GET/PUT | `/api/assets/{id}` | 详情 / 更新发布元数据 |

`folder` 类型扫描器：递归 WalkDir，按扩展名过滤，支持 glob 排除规则，增量（mtime+size 判断），文件消失标记 `missing` 而非删除记录。

### 启动逻辑松绑（lib.rs）

- `--library` 参数变为可选：路径不存在时不报错（新安装无库情况）
- 单库校验逻辑保留兼容性：只在 `library.root` 有值且与当前路径不同时才拒绝

### 测试

新增 17 个测试，全部通过；原有 32 个测试全部保持绿色（49/49）：

- DB 迁移幂等性、v1 数据保留
- LibraryRoot CRUD（创建/更新/删除/列表）
- 路径校验（不存在路径、空名称）
- WebDAV 类型强制只读
- folder 扫描发现视频文件、排除规则、增量
- 禁用库不可扫描
- 资产分页查询
- 发布元数据更新与校验（无效 privacy、超长标题）
- fast_file_hash 稳定性与大小敏感性

### 设计文档

新增 `docs/MEDIA-LIBRARY-EXPANSION.md`（424 行）：完整的产品设计文档，包含多库架构、元数据方案、API 设计、扫描器逻辑、视图设计、首次启动重设计、迁移路径、5 个已决策事项。

## 验证

`cargo fmt --check`、`cargo check --workspace --all-targets`、Rust 49 项测试、前端测试、`vue-tsc` 与 Vite 生产构建通过。本机 Windows 便携包重新构建后，确认 `u2bup-server.exe` 报告版本并提供 `/api/libraries`；已有 `.local` 数据库首次启动时迁移到 schema v2，原 163 条 LiveRec 素材仍可读取。

已知问题：尚未添加任何素材库时，所有素材库页面都显示欢迎界面；侧边栏的旧 LiveRec 入口跳到「设置 → 环境」，所以升级用户需先添加一个素材库，才能在素材库页面看到原有 LiveRec 素材。待 `--library` 自动建库与旧数据迁移完成后解决。

## 尚未实现（本 milestone 剩余）

- [x] 前端：欢迎界面（首次无库时展示）
- [x] 前端：侧边栏多库切换
- [ ] 前端：素材发布元数据编辑面板
- [ ] 前端：平铺列表视图（跨库）
- [ ] `--library` 参数的"添加一个 liverec 库"快捷逻辑
- [ ] 旧 liverec Library.assets 数据迁移进 assets_v2

---

# v0.7：Tag 触发的预览 / 正式 Release CI


日期：2026-09-22。版本号统一为 `0.7.0`。新增 [`.github/workflows/release.yml`](../.github/workflows/release.yml)：仅在推送 `v*` tag 时打包并写入 GitHub Release，日常 push/PR 仍只跑 [build.yml](../.github/workflows/build.yml) 编译检查。

- **预览 (A)**：`vX.Y.Z-preview.N` / `vX.Y.Z-rc.N` → Pre-release；各平台 unbundled zip（desktop + server，**不含 FFmpeg**）。
- **正式 (B)**：`vX.Y.Z` → 正式 Release；Windows 便携包含经 pin 校验的 FFmpeg；macOS **DMG**（暂未公证）；Linux **AppImage** + `.deb`。
- 媒体 pin：[`app/release/media/`](../app/release/media/)；下载校验与入库：`app/scripts/fetch-release-media.mjs`；制品组装：`app/scripts/package-release.mjs`。
- 文档：根 README「GitHub Releases」小节、[CROSS-PLATFORM](CROSS-PLATFORM.md) 中 A/B 边界说明。

验证：本机已跑通 Windows pin 拉取与 `package-release` 预览/正式 zip；全平台正式包以本 tag 触发的 GitHub Actions 产物为准。macOS 签名/公证与 Windows Authenticode 仍未接入。

---

# v0.6：管线模块插件化与开发者接口

日期：2026-09-22。版本号统一为 `0.6.0`。仓库根目录增加开源风格 [README](../README.md)；模块协议与 YouTube API 对照见 [WORKFLOW-MODULE-API](WORKFLOW-MODULE-API.md)。

- 内置节点以 `com.u2bup.builtin.*` 注册并保留短别名；画布模块库由注册表驱动。新增 **来源身份台账** 节点与 SQLite 台账 API。
- 支持声明式 **L1** 模块包安装/启用/卸载（`u2bup-module.json`）；预览仍为零远端副作用，写 YouTube 仅宿主 Intent。
- 文档：Intent Schema、模块 API 手册；示例标签词典包。前端模块测试与既有 workflow 回归一并保留。

验证：前端测试（含 modules）与 Rust `u2bup-core` 库测试在发版前通过；Windows 便携包目标目录 `app/dist/U2BUP-0.6.0-windows-x64/`（`U2BUP.exe` + `u2bup-server.exe`）。第三方 L2 沙箱与公网插件市场仍未开放。

---

# v0.5：已上传视频与素材库的可组合处理管线

日期：2026-09-22。版本号已统一更新为 `0.5.0`，Windows Desktop 便携包已构建于 `app/dist/U2BUP-0.5.0-windows-x64/`，入口为 `U2BUP.exe`；同包提供独立 Web 服务及 FFmpeg/FFprobe。构建脚本现在从 Cargo workspace 版本生成包名，避免下次发版遗漏目录版本。

- 根据 `app/.local/channel-research/` 的真实频道只读快照，增加 13 种可拼接节点与 4 套模板。Canvas 支持拖动、连线、条件分流和汇合、节点配置、保存、导入导出、撤销重做；本地素材与已上传视频可进入同一套预览流程。
- 已实现来源与场次候选、平台及内容多标签、标题与描述规范化、播放列表意图、封面当前帧、质量例外和版权人工标记。低置信度身份、房间归属和重复候选进入复核，AI 内容理解保留结构化接口，不伪装成已生成结果。
- 已接通本地结构化资料、YouTube 元信息及播放列表修改、关联 MP4 取帧并上传封面。远端应用前核对频道、旧值和 ETag，逐项记录结果；上传准备可继承已确认的素材资料。真实频道本轮只读，未实际提交远端批改。

验证：前端 23 项、Rust workspace 30 项测试通过；Vue 类型检查及 Vite 生产构建通过；`cargo build --release -p u2bup-desktop -p u2bup-server --locked` 通过。隔离启动便携包内的 `u2bup-server.exe`，认证快照返回 `0.5.0`，确认包内 FFmpeg/FFprobe 路径有效。Desktop 可执行文件已经生成；本轮未在真实桌面会话中重新执行 GUI 操作验收。管线行为、真实快照试跑与范围限制见 [管线设计与实现状态](CHANNEL-WORKFLOW-DESIGN.md)。

---

# v0.4：导航、选择与统一任务中心

日期：2026-09-21（用户本地时间）。已构建 Windows 桌面和 Web 服务 v0.4.0；本机 4173 服务升级后保留 163 个素材、历史任务与现有 YouTube 授权。

- 六组可展开导航、18 个子页面、hash 深链接与前进/后退。YouTube 视频、上传、变更记录和账号配置独立显示；切换页面保留上传草稿。
- 素材/频道视频支持全局清空、本页选择、反选、Shift 连选，明确展示筛选外的选中数。频道视频每页 30 项。
- `/api/tasks` 合并媒体与上传记录，任务中心提供类型/状态/文字筛选和受当前状态约束的操作。上传沿用原续传会话；任务列表无需读取 YouTube 凭据。
- 频道同步过滤重复 ID、检测分页 token 循环；成功后保存唯一数量、原始数量、重复数与完成时间。轮询账号状态使用轻量接口。
- 增加原生跨平台 CI、媒体组件校验脚本、macOS/Linux 打包配置、实验性 Docker headless。状态与剩余工作见 [跨平台构建](CROSS-PLATFORM.md)。

验证：21 项 Rust 测试、4 项选择/路由测试、workspace Clippy、TypeScript/Vite 生产构建与 Windows release 构建通过。隔离合成素材的 21 项 FFmpeg 端到端检查通过，包括取消/重试、成品复用、超长转码切割、中文路径和源文件 SHA-256 保持不变；记录位于 `app/.local/smoke/2026-09-22T02-02-39-772Z/report.json`。

浏览器验证覆盖：媒体库筛选后全局取消、1,034 条频道记录的 30 项分页、跨页/跨筛选选择、上传草稿返回后保留、任务中心与原计划入口、独立变更页、深色主题以及 840px 窄屏导航。实际检查发现并修复了本页选择数量的显示问题。真实频道操作限于只读同步，没有上传或应用远端变更。

另通过 8 项本机 headless 启动/鉴权/禁用 YouTube 边界检查（`app/.local/headless-smoke/2026-09-22T02-09-53-255Z/report.json`），以及 Node 媒体准备脚本对本机 FFmpeg/FFprobe 的复制与版本验证。headless 本机检查不等于 Docker 镜像已运行。

首轮频道调查得到 1,034 个唯一视频（原始 1,887 条记录包含重复 ID）。新版只读复测返回 1,021 个可读取视频、1,893 条原始记录和 869 条重复记录，说明两轮列表存在波动，不能视为 Studio 全量对账已通过；复测统计在 `app/.local/v04-channel-sync-verification.json`。具体开发建议见 [调研结论](CHANNEL-MANAGEMENT-FINDINGS.md)。原始标题和私密视频信息保存在 `.local/channel-research/`，不纳入 Git。

macOS/Linux/Docker 尚未实际构建验收，容器 OAuth 暂不支持；批量元信息变更尚未纳入任务中心。当前提供跨平台构建基础，不宣称全平台发行完成。

---

# v0.3.1 状态补充：2026-09-21（历史记录）

当前应用版本为 v0.3.1，主题修复证据见 [主题验证](THEME-VERIFICATION.md)。用户在本次沟通中确认：已实际测试真实 YouTube 私密上传，结果可以接受。将该项更新为「用户实测通过」，不扩大为远端批量修改、播放列表或所有网络恢复场景均已验收；本轮没有再次操作账号。

后续开发顺序、八项扩展需求及验收门槛见 [下一阶段开发规划](DEVELOPMENT-ROADMAP.md)。下方 v0.1–v0.3 为各阶段当时的历史记录，不代表新增规划已经完成。

---

# v0.3：界面主题

日期：2026-09-21。新增浅色、深色、跟随系统模式，森林、海洋、紫罗兰、琥珀、玫瑰五种主题色，减少动态效果选项，以及顶栏明暗快捷切换。设置保存在当前 WebView/浏览器的 `localStorage`，不进入业务数据库；系统模式会监听操作系统主题变化。已验证深色紫罗兰主题、顶栏切换、刷新后保留偏好及恢复默认，并通过 TypeScript/Vite 生产构建。

v0.3 便携入口：`app/dist/U2BUP-0.3.0-windows-x64/U2BUP.exe`。已在仅保留 Windows/System32 搜索路径的环境中启动包内服务，确认版本 0.3.0 且 FFmpeg/FFprobe 均解析到便携包资源目录；记录位于 `app/.local/theme-portable-verification.json`。

---

# v0.2：YouTube 与自动切割

日期：2026-09-20（本地时间）。源码已初始化 Git，首次提交 `4ffa03e` 保存 v0.1 基线；后续功能单独提交。

新增实现：

- 桌面 OAuth + PKCE + state 校验，系统浏览器授权，系统凭据存储，访问令牌刷新与撤销授权。
- 已验证成品的串行上传队列，8 MiB 分块，服务端进度查询，网络错误退避，暂停/继续，重启后手动恢复；保留上传会话防止重复创建视频。
- 频道上传列表分页同步；标题查找替换/正则/前缀、描述、标签、公开状态批量预览与应用；本地分组和筛选。提交前重新读取远端字段并检查冲突，保存逐项结果。
- 超长或超体积单文件自动生成连续切割计划，以 H.264/AAC 精确转码；成品校验通过后才进入上传候选。原片保留。

验证：15 项 Rust 测试、Clippy、TypeScript/Vite 构建通过；21 项本地端到端检查通过。125 秒测试录像按 60 秒上限生成 58.022 / 58.022 / 9.042 秒三片，原始文件 SHA-256 未变化。

模拟 YouTube 服务测试覆盖：部分接收后返回 503、配额错误中断后恢复、按服务器 Range 续传、最后一块响应丢失后的完成确认、不重复建立上传会话、输入变更拦截、远端字段冲突拦截、保留未编辑字段、成功项目不重复提交、错误 OAuth state 拒绝。模拟端点只编译进 Rust 测试，发行程序固定连接 Google 服务。

v0.2 开发时尚未用真实 OAuth 客户端完成 Google 授权、实际上传或远端批量修改验收，当时未配置用户凭据，也未上传 LiveRec 视频。后续状态已更新：2026-09-21 用户反馈真实私密上传测试通过；远端批量管理另行验收。

Windows v0.2 便携程序已在仅 Windows/System32 的搜索路径下启动验证，读取 163 个素材并使用包内 FFmpeg/FFprobe；YouTube 配置状态接口正常。本地记录：`app/.local/v02-portable-verification.json`。

v0.2 便携入口：`app/dist/U2BUP-0.2.0-windows-x64/U2BUP.exe`。配置步骤和限制见 [使用说明](../app/README.md)。本地端到端记录：`app/.local/smoke/2026-09-21T01-23-58-775Z/report.json`（UTC 时间）。

边界：本地分组不是 YouTube 播放列表；尚无播放列表编辑、远端删除、定时发布、缩略图上传。过期上传会话不会自动重建；需先核对频道是否已产生视频。切割需要转码，会耗费 CPU 且可能损失画质。macOS/Linux 未构建验证，Linux 凭据持久化也未验收。

---

# v0.1 历史构建与验证记录

日期：2026-09-20。交付范围：Rust 核心、Vue WebUI 与 Tauri Windows 桌面的本地录播管理原型。不是完整产品或完整 M1 验收通过。

## 运行入口

- Windows 便携程序：`app/dist/U2BUP-0.1.0-windows-x64/U2BUP.exe`，需保留同目录 `resources`。
- 独立 Web 服务：包内 `Start-Web.ps1 -Library <素材根目录>`。本次演示已在 `http://127.0.0.1:4173/` 启动，浏览器已完成本机会话认证。
- 源码和使用方法：[app/README.md](../app/README.md)。服务重启后使用启动时生成的新认证链接。
- 桌面与独立 Web 服务默认各自保存数据库。若希望浏览同一批任务，使用对应服务数据目录 `connection.json` 中的链接。

## 已完成的功能

素材扫描与探测缓存、XML 历史信息读取、直播间别名聚合、在线 B 站房间信息抓取、筛选和跨页批选、缩略图、MP4 预览、显示标题批量预览与修改、合并计划、真实 FFmpeg 合并、任务进度与取消/重试、成品校验和复用、SQLite 持久化及启动中断恢复。

合并按房间、历史标题、时间连续性及流参数划分；保留画幅变化的时间顺序，避免跨越未选片段。默认时长预算 11 小时 55 分钟，体积预算 250 GB，并预留封装空间。未知时长、未知时间、单文件超过预算等情况会阻止执行。当前不会自动切割超长单文件。

标题修改只更新应用显示名称，不重命名原始文件；合并写入单独输出目录。输出校验包含流参数、时长/体积及连接处抽样解码，并非全片解码。

## 验证结果

| 验证 | 结果 |
| --- | --- |
| Rust 核心与服务测试 | 9 项通过 |
| Clippy | 核心与服务 all-targets，禁止 warning，通过 |
| Vue / TypeScript / Vite | 类型检查与生产构建通过 |
| 合成素材端到端 | 15 项检查通过，包括取消后重试、成品复用、源文件变化拦截、中文和单引号路径 |
| 真实 LiveRec 扫描 | 163 个视频、83 个房间；162 个有有效时长，1 个未知时长被明确标记 |
| XML 关联 | 87 个视频提取到 XML 录制时间 |
| 在线房间资料 | 房间 76742 抓取成功，当前信息与历史信息分别保存 |
| 真实合并 | 柒禾兔 4 个片段生成 135.852 秒 MP4，4 个原始文件 SHA-256 均未变化 |
| Windows 便携包 | 仅保留 Windows/System32 搜索路径时正常启动，使用包内 FFprobe 扫描 163 个视频、包内 FFmpeg 生成缩略图 |
| WebUI | 浏览器验证素材列表、异常文件阻止执行、任务记录与成品路径 |

本机详细记录（运行产物，不纳入源码版本控制）：

- [真实合并记录](../app/.local/liverec-verification.json)
- [便携包验证记录](../app/.local/portable-verification.json)
- [端到端报告](../app/.local/smoke/2026-09-20T15-04-35-900Z/report.json)

未批量合并整个历史库。原始 LiveRec 和 biliLive-tools 上游源码未作改写。

## 后续范围

YouTube OAuth、上传及批量管理，biliLive-tools 实时录制适配，AI 辅助，单文件自动切割，弹幕时间映射，文件系统批量重命名，以及完整 M1 验收仍待实现。下一阶段应先完善历史库处理中的超长单文件拆分和缺失/移动素材管理，再接入平台发布流程。

当前仅 Windows 本机单用户验证；macOS/Linux、NSIS 安装包、远程访问和多用户未验证。数据存储暂用 SQLite 版本化 JSON 文档表，尚未实现设计文档中的完整关系型领域模型。
