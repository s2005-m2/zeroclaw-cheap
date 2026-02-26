# Video Multimodal — Learnings

## 2026-02-26 Plan Start
- IMAGE_MARKER_PREFIX = "[IMAGE:" at multimodal.rs:7 — VIDEO version mirrors this
- parse_image_markers() at multimodal.rs:53-86 — template for parse_video_markers()
- MultimodalConfig at schema.rs:548-586 — has max_images, max_image_size_mb, effective_limits()
- ProviderCapabilities at traits.rs:225-236 — has native_tool_calling, vision; needs video: bool
- supports_vision() at traits.rs:386-389 — template for supports_video()
- PreparedMessages at multimodal.rs:16-20 — has messages, contains_images; needs contains_videos
- compatible.rs MessagePart at line 330-340 — serde tag="type", rename_all="snake_case"
- openrouter.rs MessagePart at line 35-45 — identical structure to compatible.rs
- to_message_content in compatible.rs:882-907 and openrouter.rs:250-275 — parse image markers, need video extension
- Video uses URL pass-through (NOT base64) due to file size
- Windows build: cargo test --lib fails with LNK1318 PDB limit — use cargo check
- Default for MultimodalConfig at schema.rs:578-586 — must add video fields
