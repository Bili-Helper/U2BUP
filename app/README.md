# U2BUP 0.3.1

Rust + Tauri + Vue 的本机录播工作台原型。第一阶段面向现有 LiveRec 历史素材，原始录像保留，成品写入独立目录。

## 当前可用

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

biliLive-tools 实时录制集成、AI、自动弹幕时间映射、任意格式浏览器播放、文件系统批量重命名、远程或多用户访问。macOS/Linux 尚未构建验证。

原型的输出验证是流参数、时长/体积及片段连接处抽样解码，不是全片完整解码。文件变更判断使用路径、大小和 mtime，不是全库内容哈希。SQLite 初版采用版本化 JSON 文档表，后续会迁移到关系型领域表。

## YouTube 配置与操作

1. 在 Google Cloud 启用 YouTube Data API v3，配置 OAuth 同意屏幕，创建类型为「桌面应用」的 OAuth 客户端；测试状态需添加你的账号为测试用户。
2. 打开应用的 YouTube 页面，导入下载的客户端 JSON，点击「打开系统浏览器授权」。授权成功后返回应用。无需把凭据交给开发者。
3. 点击「同步全部频道视频」，筛选、勾选视频，填写变更，预览后点击确认应用。未选择修改的字段会保留。定时发布视频暂不支持批改公开状态。
4. 上传区勾选已验证的合并/切割成品，设置标题（支持 {文件名}）、描述、标签、分类、公开状态和儿童属性，再点击上传。默认私密、不通知订阅者；公开上传按钮明确标明公开状态。
5. 暂停或服务中断后点击「继续原任务」，先查询服务端已接收位置。已完成任务不会重复上传。批次失败会逐项记录，已成功项目不会再次提交。

凭据保存在系统凭据存储（Windows Credential Manager；macOS Keychain；Linux keyutils 的登录会话存储尚未验证）。SQLite 保存视频缓存、分组、批次与上传会话地址，属于本机私有数据，不应提交 Git 或对外分享。桌面版与独立 Web 服务使用不同数据目录，OAuth 配置也分别保存。

实现已通过本地协议测试，尚未用真实 OAuth 客户端完成 Google 授权、实际上传或远端修改验收。未审核的 API 项目可能只能私密上传；配额和频道上传资格以 Google 返回结果为准。上传会话过期时会保留记录并报错，不自动重新上传，以防产生重复视频。播放列表编辑、远端删除、定时发布和缩略图上传尚未实现。

协议依据：[桌面 OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)、[断点续传](https://developers.google.com/youtube/v3/guides/using_resumable_upload_protocol)、[视频字段更新规则](https://developers.google.com/youtube/v3/docs/videos/update)。

## 自动切割

超过输出时长/体积预算的文件会在计划中展示每一片的起点和时长。精确切割需要转码，速度取决于 CPU 与源视频规格；原片保留，输出为 H.264/AAC，可能有画质损失。默认时长 11 小时 55 分钟，再为单片编码延迟预留 2 秒。体积先按源平均码率估算，再限制编码峰值并校验成品；验证不合格不会登记为可上传成品。暂不提供关键帧无损切割模式。

## Windows 便携包

`dist/U2BUP-0.3.1-windows-x64/` 包含桌面程序、独立 Web 服务与 `resources/ffmpeg.exe`、`resources/ffprobe.exe`。

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

命令行参数：`--library`、`--data`、`--output`、`--port`、`--ffmpeg`、`--ffprobe`。同一个数据目录只允许一个服务实例。桌面和独立 Web 服务默认使用不同数据目录；若要查看同一任务，浏览器应连接桌面服务的 `connection.json` 链接。

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

```powershell
cargo test -p u2bup-core -p u2bup-server --locked
cargo clippy -p u2bup-core -p u2bup-server --all-targets -- -D warnings
cargo build -p u2bup-server --locked
node scripts/smoke.mjs
```

`smoke.mjs` 在 `.local/smoke/<时间>/` 创建独立的合成素材，测试真实 FFmpeg 处理、中文/单引号路径、未知时长、切换画幅、源文件 SHA-256、鉴权和原计划重试，不修改 LiveRec。每次留下报告和产物供检查。

真实 LiveRec 验收记录见 `../docs/BUILD-STATUS.md`。
