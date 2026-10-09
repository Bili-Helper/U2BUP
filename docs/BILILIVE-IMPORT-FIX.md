# biliLive-tools 目录导入修复

日期：2026-10-07；版本：0.8.0-preview.2。

## 根因与修复

1. v0.8.0-preview.1 侧边栏选中素材库后，媒体列表仍读取旧 `/api/snapshot` 的单一 LiveRec。现在使用 `/api/snapshot?libraryId=…`，扫描入口为 `/api/libraries/{id}/scan`，录播库与文件夹库都按自己的根目录扫描。
2. 原扫描器只认识 BililiveRecorder XML 属性；新增 biliLive-tools `<metadata>` 的 `video_start_time`、`room_title`、`user_name` 和 `room_id` 解析。XML 毫秒时间戳优先于未标时区的文件名。
3. 新增 `主播_YYYYMMDD_HHMMSS_标题_[比例或编码信息].mp4` 解析，并在 `merged` 上一层找唯一的起始分段 XML。原始录制 MP4 归为原片，合并 MP4 保留历史成品分类。
4. 文件夹扫描补齐 FFprobe 时长、画面、编码和流签名。未变化文件也重新解析录制资料，但缓存技术参数；保留显示标题、发布字段、标签及上传记录。
5. 新索引通过兼容层接入预览、批量标题、工作流及处理计划。读取源文件时重新核对注册库绑定并拒绝越界路径；规划保留未选素材上下文，禁止跨库混拼。
6. 切换库清空旧库选择与房间/日期筛选，丢弃迟到的旧库请求；扫描结束自动刷新列表。素材库删除请求改为 DELETE；真实媒体目录加入 Git 忽略规则。

## 真实目录盘点

本机一份 biliLive-tools 录播目录（已加入 Git 忽略，具体主播与文件名不入库）：

- 2 个主播目录。
- 31 个 MP4，全部位于 `merged`，均可解析合并文件名。
- 61 个 XML；其中 30 个视频有唯一对应的起始 XML。
- 剩余 1 个合并 MP4（`主播_YYYYMMDD_HHMMSS_标题_[9x16].mp4` 形式）缺少对应起始 XML。房间身份仅使用同目录、同主播且唯一的房间号；不借用其他场次时间、标题或弹幕。时间暂按文件名解释，保留时区提示。

关联的起始 XML 不覆盖整个合并视频，界面保留“弹幕尚未合并”提示。没有改动原始 MP4 或 XML。历史成品生成计划时需勾选“包含历史 MP4 成品”，现有规划器将其独立处理，避免再次误拼接。

## 回归验证

前端测试覆盖快速切换库时响应乱序、移除库时请求失效；Rust 测试覆盖目录扫描、XML/文件名解析、重复扫描保留编辑、预览 Range、路径绑定、旧 LiveRec 首扫兼容及未选中间片段保护。

端到端脚本 `app/scripts/smoke-multilibrary.mjs` 在隔离目录生成小视频，用真实 FFmpeg 验证扫描、XML 元信息、视频 Range、缩略图、批量显示标题、重扫保留发布字段、另一素材库中的实际合并、移除库后旧计划拒绝执行，并对源文件校验 SHA-256。可在 `app` 中执行：

```powershell
node scripts/smoke-multilibrary.mjs
```

通过 `U2BUP_TEST_SERVER`、`U2BUP_TEST_FFMPEG`、`U2BUP_TEST_FFPROBE` 可指定程序路径。实际素材仅进行只读探测、缩略图生成和计划预览，不为验收执行大体积合并。
