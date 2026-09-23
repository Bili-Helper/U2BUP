# U2BUP 0.7.0

Rust + Tauri + Vue 的本机录播工作台原型。第一阶段面向现有 LiveRec 历史素材，原始录像保留，成品写入独立目录。

下一阶段路线见 [开发规划](../docs/DEVELOPMENT-ROADMAP.md)：多平台/Docker、统一任务和分层导航、多目录媒体库、频道批处理、自动播放列表、工作流、弹幕与 UP 主投稿备份。

## 当前可用

- **处理管线 → 自定义管线**：13 种可配置模块、4 套模板，支持拖动排布、端口连线、条件分流/汇合、启停、复制、撤销/重做、JSON 导入导出和 SQLite 保存。
- 频道视频与媒体库的选中项可进入同一管线；来源/主播/房间/录制时间解析、多标签分类、标题/描述管理段落、标签合并、质量对账和版权人工标记共用规则。原始标题与已有描述保留，候选身份进入复核。
- 管线先试跑并展示字段差异、识别证据和节点轨迹，再冻结每批最多 100 项的应用计划。支持本地元信息保存、远端视频字段更新、播放列表改名/说明/成员归类，以及显式启用的新建私密列表。逐动作检查当前频道、远端变更及执行记录；完成动作不重复提交。
- 本地 MP4 播放器可选当前帧作为封面候选；已明确关联本地素材的频道视频可在确认计划后上传封面。上传准备页可选择继承管线准备的素材元信息，逐项展示实际标题与公开状态；来源资料不同的合并成品会阻止继承。

- 可展开的侧边栏子导航、独立页面和 hash 深链接，支持浏览器前进/后退。
- 媒体库与频道视频的全局取消选择、选本页、反选、Shift 连选和筛选外选择计数；频道列表分页显示，每页 30 项。
- 合并/切割与 YouTube 上传统一进入任务中心，按类型、状态与文本筛选，提供对应的取消、暂停、恢复、重试与原计划入口。
- YouTube 拆分为频道视频、上传准备、批量变更记录；账号授权移至设置。后台轻量轮询避免反复传输完整频道元信息。
- 频道同步按视频 ID 去重，记录唯一数/原始数/重复数和同步时间，重复分页标记会停止同步并保留上次完整列表。
- 本地素材扫描、FFprobe 参数缓存、XML 历史资料、房间别名聚合。
- 素材搜索、直播间/日期/类型筛选、跨页批量选择、缩略图、MP4 预览。
- B 站直播间当前资料抓取，保留历史标题，记录失败原因。
- 显示标题的模板、普通替换、正则替换，预览后应用；不改磁盘原文件名。
- 按直播间、标题、连续时间、完整探测流参数生成合并计划。画幅切换保留时间顺序。
- 时长与体积双重预算；超长/超体积单文件自动生成连续切割计划，以 H.264/AAC 转码输出并验证。未知时间或未知时长仍阻止。历史 MP4 默认排除，选择纳入时单独处理。
- 真正执行 FFmpeg 流复制合并，显示进度、取消、原计划重试；完成记录、媒体校验与抽样解码。
- SQLite 持久化素材、计划、任务与成品；启动时恢复中断状态，已登记产物会重新验证后复用。
- Tauri Windows 桌面入口与同一套本机 WebUI，认证会话按端口隔离。
- 浅色、深色、跟随系统三种显示模式，五种主题色、减少动态效果及顶栏明暗快捷切换。外观偏好保存在当前设备，系统模式会实时响应系统主题变化。
- 主题色覆盖侧边栏选中态、总览图标与边框、标签、列表选择、直播间头像、进度条和弹窗；深色工作区使用中性底色与高对比文字。颜色统一在 `web/src/theme.css` 定义，组件样式引用语义变量。

- YouTube 桌面 OAuth + PKCE、频道视频分页同步、成品串行断点上传与暂停/继续。
- 视频标题查找替换/正则/前缀、描述、标签、公开状态批量预览与应用，以及应用内分组。

## 尚未实现

biliLive-tools 实时录制集成、AI、自动弹幕时间映射、任意格式浏览器播放、文件系统批量重命名和多用户访问。macOS/Linux 构建矩阵与 Docker headless 基础已配置，但尚未实际构建验收；容器模式暂禁用 YouTube，详见 [跨平台构建说明](../docs/CROSS-PLATFORM.md)。频道只读调研结论见 [频道管理调研](../docs/CHANNEL-MANAGEMENT-FINDINGS.md)。

原型的输出验证是流参数、时长/体积及片段连接处抽样解码，不是全片完整解码。文件变更判断使用路径、大小和 mtime，不是全库内容哈希。SQLite 初版采用版本化 JSON 文档表，后续会迁移到关系型领域表。

## YouTube 配置与操作

1. 在 Google Cloud 启用 YouTube Data API v3，配置 OAuth 同意屏幕，创建类型为「桌面应用」的 OAuth 客户端；测试状态需添加你的账号为测试用户。
2. 打开「设置 → YouTube 账号」，导入客户端 JSON，点击「打开系统浏览器授权」。授权成功后返回应用。
3. 在「YouTube → 频道视频」同步，筛选并跨页勾选，点击底部「编辑选中视频」，预览后确认应用。未编辑字段保留；定时发布视频暂不支持批改公开状态。
4. 在「上传准备」勾选已验证成品，设置标题（支持 {文件名}）、描述、标签、分类、公开状态和儿童属性，再提交。默认私密、不通知订阅者，提交后自动进入任务中心。
5. 在任务中心暂停、继续或重试上传，继续前会查询服务端已接收位置。已完成任务不会重复上传。批量元信息变更的逐项结果在「批量变更记录」查看。

凭据保存在系统凭据存储（Windows Credential Manager；macOS Keychain；Linux keyutils 的登录会话存储尚未验证）。SQLite 保存视频缓存、分组、批次与上传会话地址，属于本机私有数据，不应提交 Git 或对外分享。桌面版与独立 Web 服务使用不同数据目录，OAuth 配置也分别保存。

实现已通过本地协议测试。2026-09-21 用户确认已实际测试真实 YouTube 私密上传，结果可接受；该项记为用户实测通过，远端批量修改及其他发布/恢复场景不因此视为已全部验收。未审核的 API 项目可能只能私密上传；配额和频道上传资格以 Google 返回结果为准。上传会话过期时会保留记录并报错，不自动重新上传，以防产生重复视频。播放列表编辑与当前帧封面上传已接入自定义管线并通过本地协议测试；真实频道写入仍待用户验收。远端删除和定时发布尚未实现。

协议依据：[桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[断点续传](https://developers.google.com/youtube/v3/guides/using_resumable_upload_protocol)、[视频字段更新规则](https://developers.google.com/youtube/v3/docs/videos/update)。

## 自动切割

超过输出时长/体积预算的文件会在计划中展示每一片的起点和时长。精确切割需要转码，速度取决于 CPU 与源视频规格；原片保留，输出为 H.264/AAC，可能有画质损失。默认时长 11 小时 55 分钟，再为单片编码延迟预留 2 秒。体积先按源平均码率估算，再限制编码峰值并校验成品；验证不合格不会登记为可上传成品。暂不提供关键帧无损切割模式。

## Windows 便携包

`dist/U2BUP-0.7.0-windows-x64/` 包含桌面程序、独立 Web 服务与 `resources/ffmpeg.exe`、`resources/ffprobe.exe`。

双击 `U2BUP.exe`。首次启动选择包含主播子目录的素材根目录；点击“扫描 LiveRec”。桌面数据存放于系统应用数据目录 `io.u2bup.desktop/data`，成品在同级 `exports`，根目录选择记录在 `library-root.txt`。

也可从 PowerShell 运行包内的：

```powershell
.\Start-Web.ps1 -Library 'C:\path\to\LiveRec'
```

终端会打印带本机会话令牌的启动链接。打开该链接，令牌交换为 HttpOnly 会话 cookie 后会从地址栏移除。数据目录的 `connection.json` 也保存本次启动链接；不要对外分享该文件或 `.local` 数据。

运行需要系统 WebView2。已在本机 Windows 验证；便携目录不是完成全平台安装认证的正式发行版。

## 从源码运行 WebUI

需要 Rust/MSVC、Node、FFmpeg 和 FFprobe。锁文件已纳入源码，首次安装后：

```powershell
.\Start-Web.ps1 -Build
```

后续只启动已有构建：

```powershell
.\Start-Web.ps1
```

手动构建：

```powershell
npm --prefix web ci
npm --prefix web run build
cargo build -p u2bup-server --locked
.\target\debug\u2bup-server.exe --library '..\LiveRec' --port 4173
```

Web 静态资源由 Rust 二进制嵌入，发行运行不需要 Node/Vite。开发时先构建 Web 再编译 Rust。服务默认只监听 127.0.0.1，拒绝跨来源 API 调用，不应直接暴露公网。

命令行参数：`--library`、`--data`、`--output`、`--port`、`--ffmpeg`、`--ffprobe`，以及试验性 `--headless`、`--bind`、`--public-origin`。同一个数据目录只允许一个实例。桌面与独立 Web 服务默认使用不同数据目录；查看同一任务时应连接对应 `connection.json` 的链接。

## 构建桌面和便携包

```powershell
npm --prefix web run build
cargo build -p u2bup-desktop --locked
.\target\debug\u2bup-desktop.exe --library '..\LiveRec'
```

```powershell
.\scripts\build-portable.ps1
```

`stage-media.ps1` 的默认媒体组件来源为本机 Scoop FFmpeg，可以用 `-FfmpegDir` 指定另一发行目录。生成的媒体清单包含 SHA-256 和版本。发行模式桌面拒绝静默使用 PATH 中的 FFmpeg。

`desktop/tauri.bundle.conf.json` 提供 NSIS 安装包配置；要生成安装包，先准备组件，然后在 `desktop` 目录执行 Tauri CLI build 并传入该配置。便携包构建不等于 NSIS 安装包已测试。

## 验证

前端测试需要 Node 24 或更新版本。

```powershell
npm --prefix web test
cargo test -p u2bup-core -p u2bup-server --locked
cargo clippy -p u2bup-core -p u2bup-server --all-targets -- -D warnings
cargo build -p u2bup-server --locked
node scripts/smoke.mjs
```

`smoke.mjs` 在 `.local/smoke/<时间>/` 创建独立的合成素材，测试真实 FFmpeg 处理、中文/单引号路径、未知时长、切换画幅、源文件 SHA-256、鉴权和原计划重试，不修改 LiveRec。每次留下报告和产物供检查。

真实 LiveRec 验收记录见 `../docs/BUILD-STATUS.md`。

## 自定义管线使用

1. 从「处理管线 → 自定义管线」选择模板，或从频道视频 / 媒体库批量选择后点击「加入自定义管线」。
2. 添加模块，点击源节点右侧端口，再点击目标左侧端口。连线决定运行顺序；拖动位置只改变排布。右侧配置模块，模板可保存和导出。
3. 在「输入数据」筛选、跨页选择，必要时校正来源身份、明确绑定播放列表，或关联本地 MP4 取封面。首次归列表先点击「同步播放列表」。
4. 「预览」只在本地计算。核对原值/新值及复核原因后，生成应用计划；检查冻结后的列表说明和封面，再点击「确认应用这些变更」。历史记录可查看逐项结果并重试未完成项。
5. 研究快照 `channel-snapshot.json` 可手动导入试跑，但该模式不能提交远端变更。应用远端改动需切回当前已授权频道。

版权 claim / strike 是人工档案，地区限制另行提示，不把 `licensedContent` 当作版权警告。申诉/静音/裁剪仍在 YouTube Studio 完成。AI 节点只导出结构化理解任务与提案 schema，尚未接入推理服务。当前每个视频的一次应用计划支持一个列表；上传继承仅涵盖准备好的标题、描述、标签、分类和公开状态，上传后的自动归列表/封面链路仍待接入。

设计依据、真实样本统计和后续字幕/章节/版本归档模块见 [频道管线设计](../docs/CHANNEL-WORKFLOW-DESIGN.md)；宿主 Intent 契约见 [WORKFLOW-INTENT-SCHEMA](../docs/WORKFLOW-INTENT-SCHEMA.md)；**模块功能、YouTube API 对照与第三方开发指南**见 [WORKFLOW-MODULE-API](../docs/WORKFLOW-MODULE-API.md)。声明式模块示例：`web/src/modules/examples/tag-dictionary.mjs`。UI 隔离验收脚本为 `scripts/workflow-ui-smoke.mjs`，通过 playwright-cli 对开发服务运行，所有 API 使用合成数据；不会修改真实频道。
