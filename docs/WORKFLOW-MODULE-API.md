# 管线模块接口说明（moduleApi 1）

本文描述 U2BUP **自定义管线** 当前已实现的模块能力、实现路径、所用 Google YouTube Data API v3 资源，以及第三方开发者编写声明式（L1）模块的指导。字段与行为以源码为准；宿主 Intent 清单另见 [WORKFLOW-INTENT-SCHEMA.md](WORKFLOW-INTENT-SCHEMA.md)，产品设计背景见 [CHANNEL-WORKFLOW-DESIGN.md](CHANNEL-WORKFLOW-DESIGN.md)。完整开发者手册即本文。

**版本约定**

| 项 | 值 |
| --- | --- |
| 管线图 `version` | `1` |
| 模块协议 `moduleApi` / 包内 `apiVersion` | `1` |
| 应用版本（撰写时） | `0.5.0` |
| 第三方可安装包类型 | 仅 `kind: "declarative"`（L1） |
| 内置执行逻辑 | TypeScript 预览引擎（同源 builtin），远端写入仅宿主 Rust |

**硬边界（开发者必须遵守）**

1. **管线图是 JSON 文档，不是可执行代码**（见 `workflow.rs`）。
2. **预览阶段禁止网络与任意文件 I/O**；模块只能改本地运行态记录或产出宿主认识的 Intent。
3. **第三方不能注册新的 YouTube 写操作**；只能通过已登记 Intent，由宿主 `apply` 执行。
4. Content ID / 版权申诉详情需要 Partner/Content Manager 能力，**普通 Data API 频道授权无法自动扫描版权警告**。

---

## 1. 整体架构

```text
┌─────────────────────────────────────────────────────────────┐
│ WebUI（Vue）                                                  │
│  模块库 / 画布 / 模块管理 / 预览差异 / 冻结计划                 │
└───────────────────────────┬─────────────────────────────────┘
                            │ 纯本地
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ 预览引擎 app/web/src/workflow.ts + modules/*                  │
│  ModuleRegistry → 启用模块列表                                │
│  runWorkflow(graph, records, context) → FieldDiff / Issue /   │
│  HostIntent / PlaylistIntent（零远端副作用）                   │
└───────────────────────────┬─────────────────────────────────┘
                            │ POST /api/workflows/preview → apply
                            ▼
┌─────────────────────────────────────────────────────────────┐
│ 宿主 Apply app/crates/core/src/workflow.rs                    │
│  读旧值、ETag、频道校验 → YouTube Data API / 本地 SQLite       │
└─────────────────────────────────────────────────────────────┘
```

数据流要点：

- **画布位置只影响布局**；连线决定执行顺序（DAG）。`filter` 有 `yes`/`no` 端口；多入口须经 `join`。
- `metadata` / `playlist` 节点要求上游每条路径都经过已启用的 `parse`。
- 冻结计划每批最多 **100** 条；每个视频一次计划最多 **一个** 播放列表意图。

---

## 2. 统一记录模型（模块读写对象）

预览时每条输入归一为 `NormalizedRecord`（见 `workflow.ts`）。模块通过修改字段、追加 `evidence` / `issues`、或产出 Intent 表达结果。

### 2.1 主要字段

| 字段 | 含义 | 典型写入模块 |
| --- | --- | --- |
| `id` / `kind` | 稳定 ID；`youtube` 或 `local` | input |
| `title` / `description` / `tags` | YouTube snippet 对应字段（本地素材亦有同构） | metadata, tags, L1 |
| `privacy` / `categoryId` | 可见性与分类 ID | 管线默认不改 privacy；category 可被字段 diff 带入 apply |
| `creator` / `roomId` / `platform` | 来源身份 | parse, identity, L1 |
| `recordedAt` / `recordedEnd` / `sessionTitle` / `part` | 录制时间与场次主题（≠ `publishedAt`） | parse, metadata |
| `labels` | 应用内内容标签（可再合并进 YouTube tags） | classify, L1 |
| `copyrightStatus` | `unknown` \| `clear` \| `claim` \| `strike`（人工） | copyright |
| `thumbnailCandidate` | 本地选帧候选 | thumbnail |
| `aiProposal` | Agent 任务快照（未接入时 `awaiting_agent`） | ai |
| `evidence[]` | `{ field, value, source, confidence }` | 各理解模块 |
| `issues[]` | `{ code, message, severity, nodeId? }` | 各模块 |
| `raw` | 原始 YouTube/素材 JSON（只读参考） | — |

### 2.2 运行上下文 `WorkflowContext`

| 键 | 用途 |
| --- | --- |
| `identityBindings` | 高置信证据推导的临时绑定 |
| `identityLedger` | 用户保存的来源身份台账（SQLite） |
| `playlists` | 已同步的播放列表及成员 ID |
| `records` | 同批对照（质量查重用） |
| `copyrightReviews` | 按视频 ID 的人工版权状态 |

### 2.3 能力声明 `capabilities`

模块 manifest 使用字符串白名单，便于 UI 展示与后续校验：

| 模式 | 含义 |
| --- | --- |
| `read.record` | 读取记录 |
| `write.fields:a,b` | 预览阶段可改所列字段 |
| `emit.intent:playlist.add` | 可产出对应 Intent |

预览副作用声明固定为 `sideEffect: "pure"`。真正远端副作用只出现在宿主 Intent 执行阶段。

---

## 3. 内置模块目录（功能与实现）

内置 ID 形如 `com.u2bup.builtin.<alias>`；图中可写短别名（如 `parse`）。实现位置：`app/web/src/workflow.ts`（逻辑）+ `modules/registry.ts`（注册）。

| 短名 / 完整 ID | 分组 | 功能摘要 | 实现要点 | 是否调用 YouTube API |
| --- | --- | --- | --- | --- |
| `input` | 流转 | 接收所选频道视频 / 本地素材，按 `kind:id` 去重 | 按 `config.kind` 过滤 | 否（用已缓存数据） |
| `parse` | 理解 | 从标题/描述解析主播、平台、房间、日期、场次、技术后缀 | 日历校验；`rN_M` 保留；读 `[U2BUP 元信息]` 块 | 否 |
| `identity` | 理解 | 用台账按房间匹配身份 / 别名 | 冲突 → `identity_conflict`；可 `identity.upsert` | 否（台账本地） |
| `classify` | 理解 | 平台与内容多标签 | 关键词规则；`platform` 单独字段 | 否 |
| `filter` | 流转 | 条件分流 yes/no | 空/包含/比较等算子 | 否 |
| `metadata` | 修改 | 标题模板 + 结构化描述块 | 默认模板 `【{主播}】{主题} {分片}`；描述包在 `[U2BUP 元信息]` | 否（预览）；apply 时写远端 |
| `tags` | 修改 | 合并 labels / 主播名 / 自定义 tags | 总长预算约 500 字符 | 否（预览）；apply 时写远端 |
| `playlist` | 修改 | 按平台+房间匹配列表；主播名作列表名 | 名称相似不自动绑定；可建议新建私密列表 | 否（预览）；apply 时写远端 |
| `quality` | 复核 | 极短片、同标题/同场次候选、缺字段 | 不删除、不判死重复 | 否 |
| `copyright` | 复核 | 人工 claim/strike；地区限制单独 issue | **不**把 `licensedContent` 当版权结论 | 否（读缓存 `regionRestriction`） |
| `thumbnail` | 修改 | 记录本地 MP4 选帧时间 | 无本地素材则 review | 否（预览）；apply 时 upload |
| `ai` | 理解 | 产出 Agent 任务 schema | 未接入时不伪造标题 | 否 |
| `join` | 流转 | 合并分支；同字段冲突则阻断 | 祖先写优先语义 | 否 |
| `output` | 流转 | 汇总差异与轨迹 | 每图恰好一个 | 否 |

### 3.1 描述管理块格式

`metadata` / L1 描述规则写入可重解析区块：

```text
[U2BUP 元信息]
主播：…
平台：…
录像时间：…
场次主题：…
房间号：…
…
原始标题：…
[/U2BUP 元信息]
```

`parse` 会从该块恢复字段；再次运行时替换整块，尽量保持幂等。

### 3.2 播放列表身份标记

宿主在列表说明中写入 `U2BUP identity: <platform>:<roomId>`（及平台/房间号可读行），便于再次匹配；**不以列表标题模糊匹配主播名**。

---

## 4. YouTube Data API v3：管线实际用法

OAuth 范围（连接频道时）：`https://www.googleapis.com/auth/youtube.force-ssl`  
（实现见 `youtube.rs` 授权请求。）

官方文档入口：[YouTube Data API Overview](https://developers.google.com/youtube/v3/getting-started)、[videos](https://developers.google.com/youtube/v3/docs/videos)、[playlists](https://developers.google.com/youtube/v3/docs/playlists)、[playlistItems](https://developers.google.com/youtube/v3/docs/playlistItems)、[thumbnails](https://developers.google.com/youtube/v3/docs/thumbnails)。

### 4.1 读路径（输入数据，非模块内直连）

| 场景 | API | part / 参数 | 说明 |
| --- | --- | --- | --- |
| 账号频道 | `channels.list` | `snippet,contentDetails`，`mine=true` | 取 uploads 播放列表 ID |
| 上传列表分页 | `playlistItems.list` | uploads playlist | 频道视频 ID 列表 |
| 视频元信息 | `videos.list` | `snippet,status,contentDetails,statistics` | 同步进 SQLite `yt-video` |
| 播放列表同步 | `playlists.list` + `playlistItems.list` | `snippet,status` 等 | 管线「同步播放列表」 |
| Apply 前复核 | `videos.list` / `playlists.list` | `snippet,status` 等 | 冲突检测与 ETag |

模块预览**不**直接发起上述请求，只消费 WebUI / SQLite 已有缓存。

### 4.2 写路径（仅宿主 Apply）

宿主白名单路径：`videos` \| `playlists` \| `playlistItems`（`workflow_write`），以及上传端点 `thumbnails/set`。

| 管线动作 | HTTP | API 方法 | part / 请求体要点 | 安全措施 |
| --- | --- | --- | --- | --- |
| 更新标题/描述/标签/分类 | `PUT` | `videos.update` | `snippet`：须带齐可写 snippet；`status`：仅当改 privacy | 先读当前值合并；`If-Match: etag`；校验 `channelId` |
| 改公开状态 | `PUT` | `videos.update` | `status.privacyStatus` | 有 `publishAt`（定时发布）则拒绝 |
| 新建播放列表 | `POST` | `playlists.insert` | `snippet` + `status.privacyStatus=private` | 默认私密；创建回执防重复 |
| 改列表名/说明 | `PUT` | `playlists.update` | `snippet.title/description`（保留 `defaultLanguage`） | ETag；预览后变更则要求重预览 |
| 视频加入列表 | `POST` | `playlistItems.insert` | `snippet.playlistId` + `resourceId.videoId` | 已在列表则跳过；未知结果不盲目重插 |
| 设置封面 | `POST` | `thumbnails.set` | `videoId`，`uploadType=media`，JPEG body | 校验本地帧 hash / 素材 mtime；远端封面预览后变化则拒绝 |

**字段约束（宿主校验，与 Google 限制对齐）：**

- 标题：1–100 字符，不含 `<` `>`
- 描述：≤ 5000 字节量级校验（实现按字符长度保护）
- tags：单项非空、总长度预算约 500

### 4.3 模块 ↔ API 对照

| 模块产出 | 如何落到 API |
| --- | --- |
| `title` / `description` / `tags` / `categoryId` / `privacy` 字段 diff | → `videos.update`（youtube）或本地 `workflow-metadata` + 显示标题（local） |
| `playlist.add` / `create` + 改名说明 | → `playlists.insert` / `playlists.update` / `playlistItems.insert` |
| `thumbnail.setFromLocalFrame` | → 宿主先存 JPEG，再 `thumbnails.set` |
| `identity.upsert` | → **仅** SQLite 身份台账，无 Google 调用 |
| `agent.task` / `publish.settings` | → 预览/复核占位，当前无 Google 调用 |
| `copyright` 人工状态 | → 本地记录；**无** Content ID API |

### 4.4 明确未使用 / 不可用的 Google 能力

| 能力 | 原因 |
| --- | --- |
| Content ID / claim 列表自动拉取 | 需 Content Manager / Partner API，非普通频道 scope |
| 字幕上传、章节专用 API | 仍为产品规划，未接入管线 apply |
| Analytics 效果归因 | 未授权进管线 |
| 任意自定义 REST | `workflow_write` 拒绝非白名单 path |

---

## 5. 宿主 Intent 与冻结计划

预览可产出 `HostIntent`（`modules/types.ts`）。播放列表类同时镜像为 legacy `PlaylistIntent`，供现有冻结载荷使用。

| kind | 预览 | Apply |
| --- | --- | --- |
| （字段 diff → 隐式 metadata） | 是 | `videos.update` / 本地 metadata |
| `playlist.add` / `create` / `review` | 是 | add/create 执行；review 不写入 |
| `playlist.updateSnippet` | 与列表意图一并处理 | `playlists.update` |
| `thumbnail.setFromLocalFrame` | 是 | `thumbnails.set` |
| `local.metadata.save` | 隐式 | SQLite `workflow-metadata` |
| `identity.upsert` | 是 | 台账 API / 本地存储 |
| `agent.task` | 是 | 未接 provider |
| `publish.settings` | 占位 | 未接 |

冻结：`POST /api/workflows/preview` → 不可变 `workflow-run`；应用：`POST /api/workflows/runs/{id}/apply`。未知响应标记后**禁止盲目重放**，须先核对 Studio / 同步。

---

## 6. 本机 HTTP API（模块与台账）

均需本机会话认证（启动链接 `#token=` 换 cookie）。

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| `GET` | `/api/workflows` | 草稿、运行记录、metadata、playlists、**modules**、**identityLedger**、`knownIntentKinds` |
| `POST` | `/api/workflows/drafts` | 保存管线图 |
| `POST` | `/api/workflows/preview` | 冻结应用计划（1–100 项） |
| `POST` | `/api/workflows/runs/{id}/apply` | 执行冻结计划 |
| `POST` | `/api/workflows/playlists/sync` | 同步频道播放列表与成员 |
| `GET` | `/api/workflows/modules` | 读取模块注册表 |
| `POST` | `/api/workflows/modules` | 保存本地模块列表 + `disabledBuiltinIds` |
| `POST` | `/api/workflows/modules/install` | 安装 L1 包 `{ package, trusted }` |
| `POST` | `/api/workflows/modules/{id}/enable` | `{ enabled, builtin? }` |
| `POST` | `/api/workflows/modules/{id}/uninstall` | 卸载本地模块（不可卸 builtin） |
| `GET/POST` | `/api/workflows/identity-ledger` | 身份台账；POST body `{ items:[{platform,roomId,creator,aliases?}] }` |

UI 入口：**处理管线 → 自定义管线 → 模块管理**。

---

## 7. 第三方开发指导（声明式 L1）

### 7.1 你能做什么 / 不能做什么

| 可以 | 不可以（本版本） |
| --- | --- |
| 关键词 → 内容标签 / YouTube tags | 提交任意 JS/Wasm 在服务端执行 |
| 标题模板、描述块、字段映射 | 直接 `fetch` Google 或任意 URL |
| 随包装身份台账行 | 注册新的远端写动词 |
| 安装后出现在画布，与内置节点混连 | 覆盖 `com.u2bup.builtin.*` ID |
| 导出 JSON 分享 | 保证「SEO 涨粉」或伪造 AI 理解 |

### 7.2 包格式：`u2bup-module.json`

```json
{
  "id": "com.example.my-rules",
  "version": "1.0.0",
  "apiVersion": 1,
  "kind": "declarative",
  "manifest": {
    "id": "com.example.my-rules",
    "version": "1.0.0",
    "apiVersion": 1,
    "kind": "declarative",
    "alias": "my-rules",
    "label": "我的规则",
    "description": "一句话说明预览行为",
    "group": "修改",
    "color": "#6aa84f",
    "capabilities": ["read.record", "write.fields:labels,tags"],
    "fields": [
      { "key": "includeLabels", "label": "写入 YouTube tags", "type": "checkbox" },
      { "key": "extraTags", "label": "额外标签", "type": "textarea" }
    ],
    "defaults": { "includeLabels": true, "extraTags": "" },
    "ports": ["out"],
    "sideEffect": "pure"
  },
  "rules": {
    "keywords": [
      { "label": "Dance", "keywords": ["跳舞", "dance"] }
    ],
    "includeLabelsAsTags": true,
    "extraTags": ["直播录像"],
    "titleTemplate": "【{主播}】{主题}",
    "writeTitle": false,
    "writeDescription": false,
    "onlyEmptyDescription": true,
    "fieldMaps": [
      { "from": "platform", "when": "unknown", "set": "platform", "value": "bilibili" }
    ],
    "identities": [
      {
        "platform": "bilibili",
        "roomId": "12345",
        "creator": "示例主播",
        "aliases": ["旧名"]
      }
    ]
  }
}
```

官方示例：[`app/web/src/modules/examples/tag-dictionary.mjs`](../app/web/src/modules/examples/tag-dictionary.mjs)（安装时转为同结构 JSON）。

### 7.3 `rules` 字段说明

| 字段 | 行为 |
| --- | --- |
| `keywords[]` | `title+description+tags` 子串命中 → 追加 `labels` |
| `titleTemplate` | 占位符：`{主播}{主题}{分片}{日期}{平台}{房间号}`；需主播+主题，低置信默认不改名 |
| `writeTitle` / `writeDescription` / `onlyEmptyDescription` / `allowUnconfirmed` | 与内置 metadata 同类开关；节点 config 可覆盖 |
| `descriptionFooter` / 节点 `footer` | 写入元信息块尾注 |
| `extraTags` / `includeLabelsAsTags` / `includeCreatorAsTag` | 合并进 `tags`（空字符串的 config 不会盖掉包内 `extraTags` 数组） |
| `fieldMaps` | `record[from]==when` 时设 `record[set]=value` |
| `identities` | 房间号匹配时写入 platform/creator，并 emit `identity.upsert` |

引擎实现：`app/web/src/modules/declarative.ts` → `applyDeclarativeRules`。

### 7.4 安装与验证步骤

1. 在「模块管理」选择 JSON，或 `POST /api/workflows/modules/install`。
2. 确认模块启用，出现在画布调色板。
3. 新建/编辑管线：例如 `input → parse → my-rules → tags → output`。
4. 选输入数据 → **预览**：应看到 labels/tags/标题 diff，**无**网络写请求。
5. 需要写 YouTube 时：复核 → 冻结 → 应用；标签类仍走宿主 `videos.update`。
6. 禁用内置依赖模块后，预览应报 `模块已禁用`，不得静默跳过写字段。

### 7.5 ID 与别名规范

- `id`：反向域名，≤160 字符，勿使用 `com.u2bup.builtin.` 前缀。
- `alias`：画布短名（字母数字短横线），图中 `type` 可用 alias。
- 旧图短名 `parse` 等通过 alias 表兼容完整 ID。

### 7.6 配置字段类型

`manifest.fields[]` 支持：`text` | `textarea` | `select` | `checkbox` | `number`。未知 key 会使图校验失败。`select` 必须落在 `options` 内。

---

## 8. 如何扩展自定义功能（路线图）

按推荐顺序选择扩展层，避免绕过安全模型：

### 8.1 L1 声明式（当前开放）

适合：标签词典、命名模板、简单字段映射、随包装身份行。  
交付物：单个 `u2bup-module.json`。

### 8.2 新内置模块（需改产品源码）

适合：新的解析启发式、地区独立节点、发布设置模板等。

步骤概要：

1. 在 `workflow.ts` 的 `BUILTIN_SPECS` 增加定义与 `capabilities`。
2. 在 `runWorkflow` 的 `switch (canonicalType)` 增加纯函数分支。
3. 若需远端写入：扩展 `HostIntent` + `workflow.rs` apply + Intent 文档；**不得**让预览直接调 Google。
4. 补 `app/web/tests` 行为测试与（如有）Rust apply 协议测试。
5. `registerBuiltin` 会在启动时挂上 alias。

### 8.3 L2 沙箱脚本（规划中，未开放）

计划：信任后的本地 `preview.js` / Wasm，对单条记录纯变换。仍只能 emit 已知 Intent。当前安装器拒绝非 `declarative` 包。

### 8.4 宿主新 Intent / 新 Google 资源

仅维护者可做：在 `KNOWN_INTENT_KINDS`、`workflow_write` 白名单、apply 分支中登记，并更新本文与 Intent Schema。候选方向（未实现）：字幕、章节、`publish.settings` 实写、Analytics 只读。

---

## 9. 源码索引

| 路径 | 内容 |
| --- | --- |
| `app/web/src/workflow.ts` | 预览引擎、内置逻辑、模板 |
| `app/web/src/modules/registry.ts` | 注册表、安装、启用 |
| `app/web/src/modules/declarative.ts` | L1 规则引擎 |
| `app/web/src/modules/types.ts` | Manifest / HostIntent 类型 |
| `app/web/src/modules/examples/tag-dictionary.mjs` | 示例包 |
| `app/web/src/WorkflowStudio.vue` | 工作室 UI、模块管理、冻结 |
| `app/web/src/WorkflowCanvas.vue` | 画布与启用模块库 |
| `app/crates/core/src/workflow.rs` | 草稿 / 冻结 / apply / 模块与台账 API |
| `app/crates/core/src/youtube.rs` | OAuth、`workflow_write`、`workflow_thumbnail` |
| `docs/WORKFLOW-INTENT-SCHEMA.md` | Intent 清单 |
| `app/web/tests/modules.test.mjs` | 插件与身份台账测试 |
| `app/web/tests/workflow.test.mjs` | 内置预览行为回归 |

---

## 10. 快速验收清单

- [ ] 旧模板 `standard` / `fill-empty` 无需改 type 可预览  
- [ ] 禁用 `com.u2bup.builtin.parse` 后依赖图报错  
- [ ] 安装示例标签词典后画布出现节点，预览产生 tags/labels diff  
- [ ] 预览过程无 YouTube 写请求；应用后逐项有成功/失败回执  
- [ ] 身份台账保存后，`identity` 节点能改写 creator/platform  
- [ ] 未识别 Intent / 未启用模块不会部分静默写远端  

文档与实现对齐日期：2026-09-22。若源码与本文冲突，以源码与测试为准。
