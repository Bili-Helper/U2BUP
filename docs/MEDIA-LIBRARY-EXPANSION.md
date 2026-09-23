# U2BUP 媒体库扩展产品设计

日期：2026-09-23  
基线版本：v0.7.0  
状态：设计草案，待实施

---

## 1. 背景与目标

### 1.1 当前限制

v0.7.0 的媒体库有以下硬约束：

- `Config.library` 是单一路径，启动时若检测到数据库属于不同根目录会报错
- 只支持 BililiveRecorder 格式的录播目录（`{room_id}-{主播名}/` 结构 + XML 旁路文件）
- 启动时强制要求选定录播素材库，没有"工作目录"概念
- 无远程库支持（WebDAV / OpenList）
- 媒体库视图只有一种：按直播间分组的录像列表

### 1.2 扩展目标

将 U2BUP 从"录播整理工具"扩展为**通用视频素材库 → 后处理 → 多平台发布工作台**，核心变化：

1. **多素材库**：任意数量、任意类型的素材库并存，可切换、可混用
2. **普通文件夹支持**：非录播格式的零散视频文件也能入库
3. **文件引用模式**：不强制复制源文件，工作目录只存元数据和链接
4. **丰富的库视图**：文件夹树、平铺列表、智能文件夹、LLM 分类
5. **更完整的元数据模型**：对标 YouTube 平台所需的全部字段，加用户自定义标签

---

## 2. 核心概念重构

### 2.1 素材库（Library）

一个 **Library** 是一个有名字、有类型的媒体来源。用户可以添加多个。

```
Library
├── id          TEXT  -- UUID
├── name        TEXT  -- 用户取的名字，如"B站录播"、"相机素材"
├── kind        ENUM  -- 见下表
├── path        TEXT  -- 本地绝对路径（folder / liverec）
│                        或挂载点路径（webdav / openlist）
├── display_tz  TEXT  -- 时区，如 "Asia/Shanghai"，影响日期显示
├── readonly    BOOL  -- 只读库不写入任何旁路文件
├── enabled     BOOL  -- 禁用后不参与扫描但保留已有索引
├── scan_exclude TEXT -- JSON array，glob 排除规则
├── created_at  TEXT
└── last_scanned_at TEXT
```

**Library 类型（kind）：**

| 类型 | 说明 | 扫描方式 |
|---|---|---|
| `liverec` | BililiveRecorder 录播目录 | 现有 scanner.rs（不变） |
| `folder` | 普通文件夹，任意结构 | 递归扫视频扩展名 |
| `webdav` | WebDAV 挂载路径（本地挂载点） | 同 folder，只读 |
| `openlist` | OpenList / Alist 挂载的本地路径 | 同 folder，只读 |

> **注意**：WebDAV 和 OpenList 均需用户自行在操作系统层面挂载为本地路径后再添加到 U2BUP，U2BUP 不直接实现 WebDAV 客户端协议（降低复杂度，规避凭据管理）。

### 2.2 素材（Asset）现有模型 → 扩展模型

在现有 `Asset` 结构基础上增加字段：

```
Asset（新增字段）
├── library_id       TEXT  -- 外键 → Library.id
├── source_path      TEXT  -- 文件绝对路径（可跨盘，可为远程挂载路径）
├── content_hash     TEXT  -- SHA-256[:16]，用于路径变化后 relink
├── is_linked        BOOL  -- true=引用（文件不在工作目录），false=文件在工作目录
├── file_size        INT   -- 字节数（现有 bytes 字段，确认对齐）
├── video_codec      TEXT  -- 如 "h264"、"hevc"（来自 ffprobe）
├── resolution       TEXT  -- 如 "1920x1080"
├── upload_targets   TEXT  -- JSON array: [{platform, status, url, uploaded_at}]
│
│ 以下为用户可编辑的发布元数据（对标 YouTube + 扩展）
├── pub_title        TEXT  -- 发布标题（与 display_title 分离）
├── pub_description  TEXT  -- 发布描述
├── pub_tags         TEXT  -- JSON array
├── pub_category_id  TEXT  -- YouTube 分类 ID，如 "22"
├── pub_privacy      TEXT  -- "private" | "unlisted" | "public"
├── pub_language     TEXT  -- 标题语言 BCP-47
├── pub_audio_lang   TEXT  -- 音频语言 BCP-47
├── custom_tags      TEXT  -- JSON array，用户自定义分类标签（非 YouTube tags）
└── custom_meta      TEXT  -- JSON object，预留扩展字段
```

**字段语义分离：**

| 字段 | 含义 |
|---|---|
| `title` | 原始文件名/录播场次名（不可编辑，来自扫描） |
| `display_title` | 在软件 UI 里显示的名字（用户可改） |
| `pub_title` | 上传到 YouTube 等平台用的标题（用户可改，workflow 可写入） |

### 2.3 工作目录（Workspace）

工作目录是 U2BUP 自己的数据存放位置，**不是媒体文件存放位置**：

```
~/.u2bup/  （或用户在设置里自定义）
├── u2bup.db            -- 主 SQLite 数据库
├── exports/            -- FFmpeg 成品输出默认位置
├── thumbnails/         -- workflow 封面图片缓存
├── .local/             -- channel-research 等本地数据
└── config.json         -- 工作目录路径、端口等基础配置
```

用户在**设置 → 存储**里可以修改工作目录路径。软件首次启动不强制选录播库，而是进入欢迎界面引导添加第一个素材库。

---

## 3. 元数据存储决策

**结论：全部写入 SQLite 数据库，提供可选的旁路 JSON 导出。**

详细理由：

| 场景 | 旁路文件 | SQLite |
|---|---|---|
| 远程挂载（WebDAV/OpenList 只读） | ❌ 无写权限 | ✅ |
| 万级文件启动扫描 | ❌ 每次遍历读文件，慢 | ✅ 索引查询 |
| 文件移动/重命名后 relink | ❌ 旁路文件跟着丢 | ✅ 用 content_hash 定位 |
| 并发安全 | ❌ 多进程读写冲突 | ✅ SQLite WAL 事务 |
| 用户可见/可编辑 | ✅ 记事本可打开 | ❌ 需 UI 或 CLI |
| 迁移工具导出 | — | ✅ 导出为 JSON 旁路文件（可选） |

**引用模式（is_linked=true）**：

- 用户添加一个本地文件/文件夹时，U2BUP 在数据库里建立记录，`source_path` 指向原始位置，**不复制文件**
- 扫描时如果 `source_path` 不存在但 `content_hash` 能在该库下找到新路径，自动提示 relink
- 工作目录只存数据库和 workflow 产物（封面图、FFmpeg 成品），不存原始素材

---

## 4. 素材库 API 设计（Rust 后端）

### 新增路由

```
GET  /api/libraries              -- 列出所有素材库
POST /api/libraries              -- 添加素材库
GET  /api/libraries/:id          -- 获取单个素材库信息
PUT  /api/libraries/:id          -- 修改配置（名称、排除规则等）
DEL  /api/libraries/:id          -- 删除素材库（可选：同时删除已索引的 asset）
POST /api/libraries/:id/scan     -- 触发扫描
GET  /api/libraries/:id/status   -- 扫描进度

GET  /api/assets                 -- 分页列表，支持跨库查询
     ?library_id=                -- 筛选单库
     ?q=                         -- 全文搜索
     ?tag=                       -- 按 custom_tags 筛选
     ?has_pub_meta=false         -- 只看没填过发布元数据的
     ?sort=modified_desc|size|title
     &page=&per_page=
GET  /api/assets/:id             -- 单条素材详情
PUT  /api/assets/:id/meta        -- 修改 pub_*/custom_* 字段（不改文件）
```

### 数据库迁移策略

现有 `documents(kind, id, json)` 表不动，保留现有所有数据。新增：

```sql
-- 迁移版本 2
CREATE TABLE IF NOT EXISTS libraries (
  id TEXT PRIMARY KEY,
  json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS assets_v2 (
  id TEXT PRIMARY KEY,
  library_id TEXT NOT NULL REFERENCES libraries(id),
  source_path TEXT NOT NULL,
  content_hash TEXT,
  json TEXT NOT NULL,          -- 完整 Asset 序列化（含扩展字段）
  pub_title TEXT,              -- 冗余列，用于全文搜索索引
  custom_tags TEXT,            -- JSON array 冗余，用于 tag 筛选
  modified_ms INTEGER,
  file_size INTEGER
);

CREATE INDEX IF NOT EXISTS idx_assets_library ON assets_v2(library_id);
CREATE INDEX IF NOT EXISTS idx_assets_hash ON assets_v2(content_hash);
CREATE INDEX IF NOT EXISTS idx_assets_path ON assets_v2(source_path);
```

现有 `liverec` 类型库的 asset 数据从内存状态（`Library.assets` Vec）**迁移写入** `assets_v2`，作为首次启动 v0.8 时的一次性迁移。

---

## 5. 扫描器扩展

### 5.1 `folder` 类型扫描

递归 `walkdir`，过滤视频扩展名：

```
支持扩展名：mp4 mkv mov avi flv ts m2ts wmv webm m4v 3gp
```

每个文件生成 Asset：
- `title` = 文件名（去扩展名）
- `display_title` = 同上
- `room_id` = 空（非录播不强制要求）
- `started_at` = 文件 mtime（无 XML 元数据时的 fallback）
- `content_hash` = 读前 1MB + 文件大小 组合 hash（不全文 hash，扫描快）
- ffprobe 获取 duration / codec / resolution（异步，低优先级）

### 5.2 content_hash 计算策略

不对整个文件做 SHA-256（视频文件普遍几 GB，太慢）：

```
hash = SHA-256(前 1MB 字节 + LE u64(文件大小))[:16]
```

碰撞概率极低；如需精确去重，提供"精确哈希"按需触发选项。

### 5.3 扫描增量逻辑

- 扫描时对比 `source_path` + `modified_ms` + `file_size`，三者一致则跳过
- 文件消失 → 标记 `file_missing`，不删除记录（避免误删元数据）
- 文件重新出现 → 自动恢复
- 整库离线（如 WebDAV 断开）→ 标记库状态 `offline`，不触发任何 missing 操作

---

## 6. 媒体库视图设计

### 6.1 视图类型

| 视图 | 描述 | 实现方式 |
|---|---|---|
| **文件夹树** | 按 `source_path` 实际目录层级展示 | 前端按路径分组，虚拟树组件 |
| **平铺列表** | 全部素材按时间/大小/名称排序 | 现有列表组件去掉分组 |
| **按库分组** | 每个素材库一个折叠区 | 按 `library_id` 分组 |
| **智能文件夹** | 用户保存的筛选规则 | 前端 filter 规则 JSON，对应 `/api/assets?...` 查询 |
| **未处理** | 没有 `pub_title` 或 `pub_tags` 的素材 | `/api/assets?has_pub_meta=false` |
| **待上传** | `upload_targets` 为空或 status=pending | 专项筛选 |

### 6.2 智能文件夹规则格式

```json
{
  "name": "未填标签的长视频",
  "rules": [
    { "field": "custom_tags", "op": "empty" },
    { "field": "duration_seconds", "op": "gte", "value": 3600 }
  ],
  "logic": "AND",
  "sort": "modified_desc"
}
```

规则转为后端查询参数，不在前端过滤（支持大库分页）。

### 6.3 LLM 智能分类

**触发方式**：用户选中一批素材 → 右键 → "AI 智能分类"

**输入**：
```json
{
  "assets": [
    { "path": "相对于库根目录的路径", "title": "文件名", "duration": 3600 },
    ...
  ]
}
```

**输出**：模型返回每个素材的建议 `custom_tags`，以及整体分组建议（如"这批文件像是游戏直播切片"）。

**集成点**：走现有工作流 `ai` 节点的 `agent.task` intent 机制，不新增基础设施。模型调用由用户在设置里配置 API key（OpenAI/Claude/本地 Ollama），U2BUP 只负责构造 prompt 和解析 JSON 响应。

**Prompt 模板（可用户自定义）**：
```
以下是一批视频文件的路径和文件名。请分析文件命名规律，
为每个文件推荐 2-5 个中文分类标签，并简要说明整体分类逻辑。
返回 JSON 格式：{"results": [{"path": "...", "tags": ["...", "..."]}], "summary": "..."}

文件列表：
{files}
```

---

## 7. 首次启动体验重设计

### 当前行为

启动 → 强制弹出"选择录播根目录"对话框 → 必须选择一个 BililiveRecorder 目录才能进入软件。

### 新行为

```
首次启动
  └── 欢迎界面（Welcome）
        ├── [添加录播库]    → 选择 BililiveRecorder 目录（兼容现有用户）
        ├── [添加普通文件夹] → 选择任意视频文件夹
        ├── [添加远程挂载]  → 选择已挂载的 WebDAV/OpenList 路径
        └── [先跳过]        → 进入空的媒体库界面，可以随时在侧边栏"+ 添加素材库"

非首次启动
  └── 直接进入上次使用的视图
        └── 侧边栏显示所有已添加的素材库，可切换
```

**配置文件存储**：

工作目录路径和上次选中的库 ID 存在 `config.json`（或注册表/系统配置），不在 `u2bup.db` 里，方便数据库迁移时保留。

---

## 8. 侧边栏结构调整

```
侧边栏（新版）

▼ 媒体库
  ┣ 全部素材             （跨库平铺列表）
  ┣ 文件夹树             （按实际目录）
  ┣ 未填发布元数据
  ┣ 待上传
  ┣ ─────────────────
  ┣ 📁 B站录播 [liverec]  （库切换）
  ┣ 📁 相机素材 [folder]
  ┣ 📁 WebDAV归档 [webdav]
  ┗ ＋ 添加素材库

▼ 处理
  ┣ 合并 & 切割
  ┗ 工作流

▼ 发布
  ┣ YouTube
  ┣ 上传队列
  ┗ 发布历史

▼ 任务中心

▼ 设置
```

---

## 9. 实施路线图

### Phase 1：多素材库基础（v0.8.0）

优先级最高，后续所有功能依赖。当前进度以 `v0.8.0-preview.1` 发布预览，下列项全部完成后发正式 `v0.8.0`。

**后端**
- [x] `libraries` 表 + CRUD API
- [ ] `assets_v2` 表 + 迁移脚本（表已建；现有 liverec 数据尚未迁移）
- [x] `folder` 类型扫描器
- [x] `/api/assets` 分页查询 API
- [ ] 工作目录设置（`config.json`，不再强制 `--library` 参数）
- [ ] 首次启动不强制选库，`--library` 参数保留为兼容入口（已不强制；`--library` 尚未自动建 liverec 库）

**前端**
- [x] 欢迎界面 / 添加素材库引导
- [x] 侧边栏多库切换
- [ ] 平铺列表视图（跨库）
- [ ] 素材发布元数据编辑面板（pub_title / pub_tags / pub_description）

### Phase 2：视图增强（v0.8.x）

- [ ] 文件夹树视图
- [ ] 智能文件夹（保存筛选规则）
- [ ] 素材多选 → 批量填写发布元数据
- [ ] content_hash relink（文件移动后重新关联）
- [ ] WebDAV / OpenList 挂载路径支持（只读库）

### Phase 3：LLM 集成（v0.9.0）

- [ ] 设置页：AI 服务配置（API key、模型、自定义 prompt）
- [ ] 素材选中后"AI 智能分类"入口
- [ ] 工作流 `ai` 节点接入 LLM 分类（file-based input）
- [ ] LLM 建议 → custom_tags 批量预览 → 用户确认

### Phase 4：发布平台扩展（v0.9.x）

- [ ] `upload_targets` 多平台（当前只有 YouTube）
- [ ] Bilibili 上传（参考 biliLive-tools）
- [ ] 发布状态追踪与历史记录

---

## 10. 迁移兼容性说明

### v0.7.0 → v0.8.0 升级路径

1. 软件启动时检测数据库版本（`PRAGMA user_version`）
2. 若版本为 1，执行迁移：
   - 读取现有 `Config.library` 路径
   - 在 `libraries` 表里创建一条 `kind=liverec` 的记录
   - 将内存中的 `Library.assets` 数据写入 `assets_v2` 表
   - 写入 `PRAGMA user_version=2`
3. 迁移失败时保留 v1 数据，报错提示用户手动处理
4. `--library` 命令行参数继续有效，但作为"默认添加一个 liverec 库"的快捷方式，而非强制参数

### 已有 workflow 兼容性

workflow 系统使用 `kind=youtube|local` 的 item，其中 `local` 对应本地素材。现有逻辑不变，`folder` 类型库扫描出的素材的 `kind` 也是 `local`，完全兼容。

---

## 11. 待决策事项

| # | 问题 | 决策结果 | 状态 |
|---|---|---|---|
| D1 | `config.json` 存放位置 | 两种都支持，自动检测（见下） | ✅ 已确认 |
| D2 | `assets_v2` 与 `documents` 并存还是彻底替换 | 并存，逐步迁移 | ✅ 已确认 |
| D3 | `folder` 扫描的 ffprobe 调用策略 | 异步后台，扫描完先展示列表，probe 结果陆续填充 | ✅ 已确认 |
| D4 | LLM 调用 API 格式 | 支持 OpenAI 兼容 API（Ollama / 本地模型均可） | ✅ 已确认 |
| D5 | WebDAV 是否内置客户端 | 暂不，要求用户自行挂载为本地路径 | ✅ 已确认 |

### D1 详细说明：config.json 自动检测逻辑

启动时按以下顺序寻找 config.json，找到即用，找不到则在该位置新建：

```
1. exe 同目录下的 portable.flag 文件是否存在？
   是 → 便携模式：config.json 在 exe 同目录
   否 ↓

2. exe 同目录下是否存在 config.json（历史便携安装）？
   是 → 便携模式：继续使用该位置
   否 ↓

3. 标准安装模式：
   Windows  → %APPDATA%\U2BUP\config.json
   macOS    → ~/Library/Application Support/U2BUP/config.json
   Linux    → ~/.config/u2bup/config.json
```

**便携包打包时**：在 zip 里放一个空的 `portable.flag` 文件即可强制便携模式，用户解压到哪里就用哪里的配置，数据不会跑到 `%APPDATA%`。

**config.json 内容**（最小集，其余配置存 SQLite）：

```json
{
  "workspace": "C:\\Users\\Admin\\.u2bup",
  "last_library_id": "abc123",
  "port": 4173
}
```

`workspace` 指向 SQLite 数据库和成品目录，**不是媒体文件目录**。便携模式下 `workspace` 默认为 exe 同目录下的 `.local/`，与现有 v0.7.0 行为一致。

