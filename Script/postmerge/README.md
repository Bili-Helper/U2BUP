# Bilive 录播合并工具

脚本: `merge_bilive.ps1`

## 功能

自动按直播场次(Session)和分辨率比例合并 Bilive 录制的 FLV、TS、MKV 片段为 MP4 文件。

## 特性

- **按直播会话分组** — 同标题、时间连续的文件归为同一场直播
- **跨午夜处理** — 跨日期的同一场直播自动合并
- **按分辨率比例拆分** — PK/切流造成的竖屏(9:16)↔横屏(16:9)变化自动拆分为独立文件，避免 YouTube 上传后拉伸
- **完整流配置隔离** — 相同比例但分辨率、编码 Profile、像素格式或音频参数不同的片段也会分开，避免 MP4 中途变规格
- **重连标题归一化** — 自动移除录制器追加的 `_PART000` 等后缀，避免同一直播被误拆
- **输出名防碰撞** — 文件名包含开播时间，同日同标题的多场直播不会互相跳过
- **文件名日期 = 真实直播时间** — 直接解析文件名，不受文件系统时区影响
- **流复制** — `ffmpeg -c copy`，无损、极速（无转码损耗）
- **多容器输入** — 同时扫描 FLV、TS、MKV，不遗漏录制器切换容器后留下的片段
- **临时文件验证** — 合并结果通过 ffprobe 检查后才正式落盘
- **支持安全续跑** — 已验证落盘的输出自动跳过，空间检查只计算未完成任务

## 使用方法

### 处理单个主播目录

```powershell
# 预览（不实际执行）
pwsh .\merge_bilive.ps1 -SourceDir "C:\Users\Administrator\Videos\Bilive" -SpecificDir "主播目录名" -DryRun

# 实际执行
pwsh .\merge_bilive.ps1 -SourceDir "C:\Users\Administrator\Videos\Bilive" -SpecificDir "主播目录名"
```

### 处理全部主播目录

```powershell
pwsh .\merge_bilive.ps1 -SourceDir "C:\Users\Administrator\Videos\Bilive"
```

### 输出到其他盘（节省磁盘空间）

```powershell
pwsh .\merge_bilive.ps1 -SourceDir "C:\Users\Administrator\Videos\Bilive" -SpecificDir "主播目录名" -OutputDir "D:\Bilive_merged\主播目录名"
```

### 参数说明

| 参数 | 说明 | 默认 |
|---|---|---|
| `-SourceDir` | Bilive 根目录（必填） | — |
| `-SpecificDir` | 只处理某个子目录 | 全部目录 |
| `-OutputDir` | 合并输出目录 | `SourceDir\主播名\merged\` |
| `-DryRun` | 仅预览，不合并 | 不启用 |
| `-MaxGapSeconds` | 同一场直播内文件最大间隔(秒) | 1800 (30分钟) |
| `-MaxDurationSeconds` | 单个输出文件最大时长(秒) | 21600 (6小时) |
| `-MinSizeKB` | 跳过小于此大小的录制碎片 | 512 |
| `-MinDurationSeconds` | 跳过短于此时长的异常片段 | 1 |
| `-MinFreeSpaceGB` | 合并后要求输出盘至少保留的空间 | 10 |

## 输出文件名格式

```
{主播名}_{日期范围}_{开播时间}_{标题}_[{比例标签}][_partN].mp4
```

示例:
- `冻泥不是冰的_20260805_004707_无人_无流水_无大哥_[9x16].mp4`
- `冻泥不是冰的_20260807-20260808_225820_无人_无流水_无大哥_[r1.2].mp4`

## 依赖

- PowerShell 7 (pwsh)
- ffmpeg (已安装: scoop)

## 合并 `_partN` 后处理

主合并完成后，可把同一场、同一画幅的 `_part1`、`_part2` 等继续合成一个文件：

```powershell
pwsh .\merge_parts.ps1 -SourceDir "C:\path\to\主播目录" -DryRun
pwsh .\merge_parts.ps1 -SourceDir "C:\path\to\主播目录"
```

默认把验证成功后的旧 part 移入 `merged\parts_backup`。确认无需保留时可使用 `-DeleteParts` 直接删除。
