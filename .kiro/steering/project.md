# U2BUP 项目说明

## 项目定位
U2BUP 是一个本地工作台，把 B 站直播录播库（LiveRec 格式）的归档、FFmpeg 后处理（合并/切割）和 YouTube 上传管理统一在一个桌面应用里。

## 技术栈
- **后端**：Rust + Axum + Tokio + rusqlite（SQLite 存状态）
- **前端**：Vue 3 + Vite + Lucide 图标
- **桌面壳**：Tauri v2（同时提供 `u2bup-server.exe` 无界面版本）
- **外部工具**：FFmpeg（合并/切割/probe）、YouTube Data API v3（OAuth PKCE）

## 目录结构
```
app/              主应用（Rust + Vue + Tauri）
  crates/         Rust workspace crates
  web/            Vue 前端
  desktop/        Tauri 桌面配置
  scripts/        构建脚本
  release/        发布资产（媒体、FFmpeg pins）
  .local/         本地数据（gitignore，含 channel-research）
docs/             设计文档和路线图
LiveRec/          实际的录播库目录（B 站直播录像）
biliLive-tools/   上游参考实现（不打包进 v0.7）
Script/           辅助脚本（postmerge 等）
```

## 当前版本：v0.7.0

### 已完成功能
- 媒体库扫描（LiveRec 树 + XML 元数据）
- 合并 & 切割（FFmpeg concat / H.264 cut plans）
- YouTube OAuth PKCE + 断点续传上传队列
- 工作流画布（管道节点 parse→classify→title→tags→playlists→quality→thumbnail）
- 模块插件系统（moduleApi 1，声明式 L1 JSON pack）
- 亮/暗/系统主题 + accent themes

### 明确未完成
- biliLive-tools 实时录制集成
- L2 沙盒 JS/Wasm 模块运行时
- Content ID 扫描、字幕/章节自动应用
- macOS/Linux/Docker 正式支持

## 本地开发
```powershell
cd app
.\Start-Web.ps1 -Build   # 首次：npm + vite + cargo
.\Start-Web.ps1          # 后续启动
```

## 关键文档
- `docs/DEVELOPMENT-ROADMAP.md` — 路线图
- `docs/PRODUCT-ARCHITECTURE.md` — 架构设计
- `docs/WORKFLOW-MODULE-API.md` — 模块 API
- `docs/CHANNEL-WORKFLOW-DESIGN.md` — Channel 批处理设计
- `docs/BUILD-STATUS.md` — 每版本验证日志
- `app/README.md` — 应用构建 runbook

## 关键约定
- 直播录像数据在 `LiveRec/` 目录，格式为 `{room_id}-{主播名}/`
- channel-research 数据（YouTube 元数据分析）在 `app/.local/channel-research/`，**不提交到 git**
- CI/CD 通过 tag 触发 release（格式：`vX.Y.Z` 或 `vX.Y.Z-preview.N`）
- 不直接 push 到 main/master；走 PR 流程
