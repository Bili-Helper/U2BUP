# Channel Research Pipeline 状态

## 数据概览（2026-09-22 分析）
- 总视频数：1,034
- 标签空缺率：100%
- 描述空缺率：89%
- 地区限制视频：568 条（主要 RU/BY，音乐版权问题）
- 标题格式模式：7 种

## 管道模块优先级

### P0（立即开始）
- **M01** 标题解析器：从标题提取主播名、日期、内容分类
- **M02** 主播身份注册表：room_id → 主播信息映射，需人工验证

### P1
- **M03** 标签生成：基于分类 + 主播信息批量生成标签
- **M04** 标题 SEO 改写：格式化为 YouTube 友好标题
- **M05** 描述自动填写：模板化描述生成

### P2
- Playlist read-sync（频道已有播放列表同步）
- **M07** 版权/地区限制审查：标记 RU/BY 受限视频处理策略

### P3（暂缓）
- **M06** 播放列表分配：等待 Playlist API 在界面暴露
- **M08** 异常检测
- **M09** 内容分类
- **M10** 语言补全

## 数据路径
- 原始数据：`app/.local/channel-research/`（不提交 git）
- summary.json / pattern-summary.json / REPORT.md
- private-source-candidates.json / private-patterns.json
