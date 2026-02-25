# Draft: Lark Channel Feature Superset

## Requirements (confirmed)
1. Cron delivery — 推送到飞书，支持 hook
2. Draft 流式更新 — CardKit 卡片实体
3. Typing 指示器 — CardKit "处理中" 卡片
4. 文件发送 — file/audio/video/media
5. 接收附件 — file_key 解析 + 下载
6. 飞书文档双向配置同步 — 完全双向实时

## Hook 决策
- 新增两个 modifying hook：on_cron_delivery + on_docs_sync_notify
- 可修改 channel/recipient/content，可 Cancel 阻止推送
- cron delivery 发送前走 on_cron_delivery hook
- docs sync 通知前走 on_docs_sync_notify hook
- 发送后触发 fire_message_sent（复用现有 void hook）

## 文档同步决策
- 绑定范围：config.toml + IDENTITY.md + SOUL.md + USER.md + AGENTS.md + 用户自定义 + zeroclaw 动态添加
- 同步方向：完全双向实时
- 呈现形式：代码块
- 需写权限/事件订阅指南文档

## 技术方案
- CardKit：POST /cardkit/v1/card → PUT /cardkit/v1/card/{id}（sequence 递增）
- 文件上传：POST /im/v1/files
- 文件下载：GET /im/v1/messages/{id}/resources/{key}?type=image|file
- Docs API：GET /docx/v1/documents/{id}/raw_content, PATCH blocks/batch_update
- 变更事件：drive.file.edit_v1
