# Video Multimodal — Decisions

## 2026-02-26 Plan Start
- Video markers use [VIDEO:url] format, mirroring [IMAGE:url]
- Video goes URL pass-through only — no base64 encoding/normalization
- No changes to Ollama, Anthropic, or Gemini providers
- No new external dependencies
- No fps/min_pixels/max_pixels Qwen-specific params
- DashScope format: {"type": "video_url", "video_url": {"url": "..."}}
