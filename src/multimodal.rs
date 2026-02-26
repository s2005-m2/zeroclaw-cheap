use crate::config::{build_runtime_proxy_client_with_timeouts, MultimodalConfig};
use crate::providers::ChatMessage;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use std::path::Path;

const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";
const VIDEO_MARKER_PREFIX: &str = "[VIDEO:";
const ALLOWED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

const ALLOWED_VIDEO_MIME_TYPES: &[&str] = &[
    "video/mp4",
    "video/webm",
    "video/quicktime",
    "video/x-matroska",
    "video/x-msvideo",
];

#[derive(Debug, Clone)]
pub struct PreparedMessages {
    pub messages: Vec<ChatMessage>,
    pub contains_images: bool,
    pub contains_videos: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MultimodalError {
    #[error("multimodal image limit exceeded: max_images={max_images}, found={found}")]
    TooManyImages { max_images: usize, found: usize },

    #[error("multimodal video limit exceeded: max_videos={max_videos}, found={found}")]
    TooManyVideos { max_videos: usize, found: usize },

    #[error("multimodal image size limit exceeded for '{input}': {size_bytes} bytes > {max_bytes} bytes")]
    ImageTooLarge {
        input: String,
        size_bytes: usize,
        max_bytes: usize,
    },

    #[error("multimodal image MIME type is not allowed for '{input}': {mime}")]
    UnsupportedMime { input: String, mime: String },

    #[error("multimodal remote image fetch is disabled for '{input}'")]
    RemoteFetchDisabled { input: String },

    #[error("multimodal image source not found or unreadable: '{input}'")]
    ImageSourceNotFound { input: String },

    #[error("invalid multimodal image marker '{input}': {reason}")]
    InvalidMarker { input: String, reason: String },

    #[error("failed to download remote image '{input}': {reason}")]
    RemoteFetchFailed { input: String, reason: String },

    #[error("failed to read local image '{input}': {reason}")]
    LocalReadFailed { input: String, reason: String },
}

pub fn parse_image_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(IMAGE_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + IMAGE_MARKER_PREFIX.len();
        let Some(rel_end) = content[marker_start..].find(']') else {
            cleaned.push_str(&content[start..]);
            cursor = content.len();
            break;
        };

        let end = marker_start + rel_end;
        let candidate = content[marker_start..end].trim();

        if candidate.is_empty() {
            cleaned.push_str(&content[start..=end]);
        } else {
            refs.push(candidate.to_string());
        }

        cursor = end + 1;
    }

    if cursor < content.len() {
        cleaned.push_str(&content[cursor..]);
    }

    (cleaned.trim().to_string(), refs)
}

pub fn parse_video_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(VIDEO_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + VIDEO_MARKER_PREFIX.len();
        let Some(rel_end) = content[marker_start..].find(']') else {
            cleaned.push_str(&content[start..]);
            cursor = content.len();
            break;
        };

        let end = marker_start + rel_end;
        let candidate = content[marker_start..end].trim();

        if candidate.is_empty() {
            cleaned.push_str(&content[start..=end]);
        } else {
            refs.push(candidate.to_string());
        }

        cursor = end + 1;
    }

    if cursor < content.len() {
        cleaned.push_str(&content[cursor..]);
    }

    (cleaned.trim().to_string(), refs)
}

pub fn count_video_markers(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| parse_video_markers(&m.content).1.len())
        .sum()
}

pub fn contains_video_markers(messages: &[ChatMessage]) -> bool {
    count_video_markers(messages) > 0
}

pub fn count_image_markers(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| parse_image_markers(&m.content).1.len())
        .sum()
}

pub fn contains_image_markers(messages: &[ChatMessage]) -> bool {
    count_image_markers(messages) > 0
}

pub fn extract_ollama_image_payload(image_ref: &str) -> Option<String> {
    if image_ref.starts_with("data:") {
        let comma_idx = image_ref.find(',')?;
        let (_, payload) = image_ref.split_at(comma_idx + 1);
        let payload = payload.trim();
        if payload.is_empty() {
            None
        } else {
            Some(payload.to_string())
        }
    } else {
        Some(image_ref.trim().to_string()).filter(|value| !value.is_empty())
    }
}

pub async fn prepare_messages_for_provider(
    messages: &[ChatMessage],
    config: &MultimodalConfig,
) -> anyhow::Result<PreparedMessages> {
    let (max_images, max_image_size_mb) = config.effective_limits();
    let max_bytes = max_image_size_mb.saturating_mul(1024 * 1024);
    let (max_videos, _max_video_size_mb) = config.effective_video_limits();

    let found_images = count_image_markers(messages);
    if found_images > max_images {
        return Err(MultimodalError::TooManyImages {
            max_images,
            found: found_images,
        }
        .into());
    }

    let found_videos = count_video_markers(messages);
    if found_videos > max_videos {
        return Err(MultimodalError::TooManyVideos {
            max_videos,
            found: found_videos,
        }
        .into());
    }

    if found_images == 0 && found_videos == 0 {
        return Ok(PreparedMessages {
            messages: messages.to_vec(),
            contains_images: false,
            contains_videos: false,
        });
    }

    let remote_client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut normalized_messages = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            normalized_messages.push(message.clone());
            continue;
        }

        let (text_after_images, image_refs) = parse_image_markers(&message.content);
        let (cleaned_text, video_refs) = parse_video_markers(&text_after_images);

        if image_refs.is_empty() && video_refs.is_empty() {
            normalized_messages.push(message.clone());
            continue;
        }

        let mut normalized_image_refs = Vec::with_capacity(image_refs.len());
        for reference in &image_refs {
            let data_uri =
                normalize_image_reference(reference, config, max_bytes, &remote_client).await?;
            normalized_image_refs.push(data_uri);
        }

        let content = compose_multimodal_message(&cleaned_text, &normalized_image_refs, &video_refs);
        normalized_messages.push(ChatMessage {
            role: message.role.clone(),
            content,
        });
    }

    Ok(PreparedMessages {
        messages: normalized_messages,
        contains_images: found_images > 0,
        contains_videos: found_videos > 0,
    })
}

fn compose_multimodal_message(text: &str, data_uris: &[String], video_urls: &[String]) -> String {
    let mut content = String::new();
    let trimmed = text.trim();

    if !trimmed.is_empty() {
        content.push_str(trimmed);
        content.push_str("\n\n");
    }

    for (index, data_uri) in data_uris.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        content.push_str(IMAGE_MARKER_PREFIX);
        content.push_str(data_uri);
        content.push(']');
    }

    for video_url in video_urls {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(VIDEO_MARKER_PREFIX);
        content.push_str(video_url);
        content.push(']');
    }

    content
}

async fn normalize_image_reference(
    source: &str,
    config: &MultimodalConfig,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    if source.starts_with("data:") {
        return normalize_data_uri(source, max_bytes);
    }

    if source.starts_with("http://") || source.starts_with("https://") {
        if !config.allow_remote_fetch {
            return Err(MultimodalError::RemoteFetchDisabled {
                input: source.to_string(),
            }
            .into());
        }

        return normalize_remote_image(source, max_bytes, remote_client).await;
    }

    normalize_local_image(source, max_bytes).await
}

fn normalize_data_uri(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let Some(comma_idx) = source.find(',') else {
        return Err(MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: "expected data URI payload".to_string(),
        }
        .into());
    };

    let header = &source[..comma_idx];
    let payload = source[comma_idx + 1..].trim();

    if !header.contains(";base64") {
        return Err(MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: "only base64 data URIs are supported".to_string(),
        }
        .into());
    }

    let mime = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    validate_mime(source, &mime)?;

    let decoded = STANDARD
        .decode(payload)
        .map_err(|error| MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: format!("invalid base64 payload: {error}"),
        })?;

    validate_size(source, decoded.len(), max_bytes)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(decoded)))
}

async fn normalize_remote_image(
    source: &str,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    let response = remote_client.get(source).send().await.map_err(|error| {
        MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: format!("HTTP {status}"),
        }
        .into());
    }

    if let Some(content_length) = response.content_length() {
        let content_length = usize::try_from(content_length).unwrap_or(usize::MAX);
        validate_size(source, content_length, max_bytes)?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let bytes = response
        .bytes()
        .await
        .map_err(|error| MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime = detect_mime(None, bytes.as_ref(), content_type.as_deref()).ok_or_else(|| {
        MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        }
    })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

async fn normalize_local_image(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let path = Path::new(source);
    if !path.exists() || !path.is_file() {
        return Err(MultimodalError::ImageSourceNotFound {
            input: source.to_string(),
        }
        .into());
    }

    let metadata =
        tokio::fs::metadata(path)
            .await
            .map_err(|error| MultimodalError::LocalReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_size(
        source,
        usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        max_bytes,
    )?;

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| MultimodalError::LocalReadFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime =
        detect_mime(Some(path), &bytes, None).ok_or_else(|| MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn validate_size(source: &str, size_bytes: usize, max_bytes: usize) -> anyhow::Result<()> {
    if size_bytes > max_bytes {
        return Err(MultimodalError::ImageTooLarge {
            input: source.to_string(),
            size_bytes,
            max_bytes,
        }
        .into());
    }

    Ok(())
}

fn validate_mime(source: &str, mime: &str) -> anyhow::Result<()> {
    if ALLOWED_IMAGE_MIME_TYPES.contains(&mime) {
        return Ok(());
    }

    Err(MultimodalError::UnsupportedMime {
        input: source.to_string(),
        mime: mime.to_string(),
    }
    .into())
}

fn detect_mime(
    path: Option<&Path>,
    bytes: &[u8],
    header_content_type: Option<&str>,
) -> Option<String> {
    if let Some(header_mime) = header_content_type.and_then(normalize_content_type) {
        return Some(header_mime);
    }

    if let Some(path) = path {
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(mime) = mime_from_extension(ext) {
                return Some(mime.to_string());
            }
        }
    }

    mime_from_magic(bytes).map(ToString::to_string)
}

fn normalize_content_type(content_type: &str) -> Option<String> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else {
        Some(mime)
    }
}

fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png");
    }

    if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }

    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }

    if bytes.len() >= 2 && bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }

    None
}

pub fn video_mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        "mov" => Some("video/quicktime"),
        "mkv" => Some("video/x-matroska"),
        "avi" => Some("video/x-msvideo"),
        _ => None,
    }
}

pub fn video_mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && &bytes[4..8] == b"ftyp" {
        return Some("video/mp4");
    }

    if bytes.len() >= 4 && bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some("video/webm");
    }

    None
}

pub fn validate_video_mime(source: &str, mime: &str) -> anyhow::Result<()> {
    if ALLOWED_VIDEO_MIME_TYPES.contains(&mime) {
        return Ok(());
    }

    Err(MultimodalError::UnsupportedMime {
        input: source.to_string(),
        mime: mime.to_string(),
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_image_markers_extracts_multiple_markers() {
        let input = "Check this [IMAGE:/tmp/a.png] and this [IMAGE:https://example.com/b.jpg]";
        let (cleaned, refs) = parse_image_markers(input);

        assert_eq!(cleaned, "Check this  and this");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], "/tmp/a.png");
        assert_eq!(refs[1], "https://example.com/b.jpg");
    }

    #[test]
    fn parse_image_markers_keeps_invalid_empty_marker() {
        let input = "hello [IMAGE:] world";
        let (cleaned, refs) = parse_image_markers(input);

        assert_eq!(cleaned, "hello [IMAGE:] world");
        assert!(refs.is_empty());
    }

    #[tokio::test]
    async fn prepare_messages_normalizes_local_image_to_data_uri() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("sample.png");

        // Minimal PNG signature bytes are enough for MIME detection.
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
        )
        .unwrap();

        let messages = vec![ChatMessage::user(format!(
            "Please inspect this screenshot [IMAGE:{}]",
            image_path.display()
        ))];

        let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
            .await
            .unwrap();

        assert!(prepared.contains_images);
        assert_eq!(prepared.messages.len(), 1);

        let (cleaned, refs) = parse_image_markers(&prepared.messages[0].content);
        assert_eq!(cleaned, "Please inspect this screenshot");
        assert_eq!(refs.len(), 1);
        assert!(refs[0].starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn prepare_messages_rejects_too_many_images() {
        let messages = vec![ChatMessage::user(
            "[IMAGE:/tmp/1.png]\n[IMAGE:/tmp/2.png]".to_string(),
        )];

        let config = MultimodalConfig {
            max_images: 1,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };

        let error = prepare_messages_for_provider(&messages, &config)
            .await
            .expect_err("should reject image count overflow");

        assert!(error
            .to_string()
            .contains("multimodal image limit exceeded"));
    }

    #[tokio::test]
    async fn prepare_messages_rejects_remote_url_when_disabled() {
        let messages = vec![ChatMessage::user(
            "Look [IMAGE:https://example.com/img.png]".to_string(),
        )];

        let error = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
            .await
            .expect_err("should reject remote image URL when fetch is disabled");

        assert!(error
            .to_string()
            .contains("multimodal remote image fetch is disabled"));
    }

    #[tokio::test]
    async fn prepare_messages_rejects_oversized_local_image() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("big.png");

        let bytes = vec![0u8; 1024 * 1024 + 1];
        std::fs::write(&image_path, bytes).unwrap();

        let messages = vec![ChatMessage::user(format!(
            "[IMAGE:{}]",
            image_path.display()
        ))];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 1,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };

        let error = prepare_messages_for_provider(&messages, &config)
            .await
            .expect_err("should reject oversized local image");

        assert!(error
            .to_string()
            .contains("multimodal image size limit exceeded"));
    }

    #[test]
    fn extract_ollama_image_payload_supports_data_uris() {
        let payload = extract_ollama_image_payload("data:image/png;base64,abcd==")
            .expect("payload should be extracted");
        assert_eq!(payload, "abcd==");
    }

    #[test]
    fn parse_video_markers_extracts_multiple_markers() {
        let input = "Check [VIDEO:https://example.com/v.mp4] and [VIDEO:https://example.com/w.webm]";
        let (cleaned, refs) = parse_video_markers(input);
        assert_eq!(cleaned, "Check  and");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0], "https://example.com/v.mp4");
        assert_eq!(refs[1], "https://example.com/w.webm");
    }

    #[test]
    fn parse_video_markers_keeps_empty_marker() {
        let input = "hello [VIDEO:] world";
        let (cleaned, refs) = parse_video_markers(input);
        assert_eq!(cleaned, "hello [VIDEO:] world");
        assert!(refs.is_empty());
    }

    #[test]
    fn video_and_image_markers_coexist() {
        let input = "See [IMAGE:a.png] and [VIDEO:v.mp4] here";
        let (img_cleaned, img_refs) = parse_image_markers(input);
        let (vid_cleaned, vid_refs) = parse_video_markers(input);

        assert_eq!(img_refs.len(), 1);
        assert_eq!(img_refs[0], "a.png");
        assert!(img_cleaned.contains("[VIDEO:v.mp4]"));

        assert_eq!(vid_refs.len(), 1);
        assert_eq!(vid_refs[0], "v.mp4");
        assert!(vid_cleaned.contains("[IMAGE:a.png]"));
    }

    #[test]
    fn video_mime_from_extension_maps_correctly() {
        assert_eq!(video_mime_from_extension("mp4"), Some("video/mp4"));
        assert_eq!(video_mime_from_extension("webm"), Some("video/webm"));
        assert_eq!(video_mime_from_extension("mov"), Some("video/quicktime"));
        assert_eq!(video_mime_from_extension("mkv"), Some("video/x-matroska"));
        assert_eq!(video_mime_from_extension("avi"), Some("video/x-msvideo"));
        assert_eq!(video_mime_from_extension("txt"), None);
    }

    #[test]
    fn video_mime_from_magic_detects_mp4_ftyp() {
        let bytes: &[u8] = &[0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p'];
        assert_eq!(video_mime_from_magic(bytes), Some("video/mp4"));
    }

    #[test]
    fn video_mime_from_magic_detects_webm_ebml() {
        let bytes: &[u8] = &[0x1a, 0x45, 0xdf, 0xa3];
        assert_eq!(video_mime_from_magic(bytes), Some("video/webm"));
    }

    #[test]
    fn validate_video_mime_accepts_valid_rejects_invalid() {
        assert!(validate_video_mime("test.mp4", "video/mp4").is_ok());
        assert!(validate_video_mime("test.png", "image/png").is_err());
    }

    #[tokio::test]
    async fn prepare_messages_video_url_passthrough() {
        let messages = vec![ChatMessage::user(
            "Watch this [VIDEO:https://example.com/v.mp4]".to_string(),
        )];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };
        let prepared = prepare_messages_for_provider(&messages, &config)
            .await
            .unwrap();
        assert!(prepared.contains_videos);
        assert!(!prepared.contains_images);
        assert_eq!(prepared.messages.len(), 1);
        let content = &prepared.messages[0].content;
        assert!(content.contains("[VIDEO:https://example.com/v.mp4]"));
        assert!(content.contains("Watch this"));
    }
    #[tokio::test]
    async fn prepare_messages_rejects_too_many_videos() {
        let messages = vec![ChatMessage::user(
            "[VIDEO:https://a.mp4]\n[VIDEO:https://b.mp4]\n[VIDEO:https://c.mp4]".to_string(),
        )];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };
        let error = prepare_messages_for_provider(&messages, &config)
            .await
            .expect_err("should reject video count overflow");
        assert!(error.to_string().contains("multimodal video limit exceeded"));
    }
    #[tokio::test]
    async fn prepare_messages_mixed_image_and_video() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("sample.png");
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
        )
        .unwrap();
        let messages = vec![ChatMessage::user(format!(
            "Look [IMAGE:{}] and [VIDEO:https://example.com/v.mp4]",
            image_path.display()
        ))];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };
        let prepared = prepare_messages_for_provider(&messages, &config)
            .await
            .unwrap();
        assert!(prepared.contains_images);
        assert!(prepared.contains_videos);
        assert_eq!(prepared.messages.len(), 1);
        let content = &prepared.messages[0].content;
        let (after_images, image_refs) = parse_image_markers(content);
        assert_eq!(image_refs.len(), 1);
        assert!(image_refs[0].starts_with("data:image/png;base64,"));
        let (_, video_refs) = parse_video_markers(content);
        assert_eq!(video_refs.len(), 1);
        assert_eq!(video_refs[0], "https://example.com/v.mp4");
        // cleaned text should contain "Look" somewhere
        assert!(after_images.contains("Look") || content.contains("Look"));
    }

    #[tokio::test]
    async fn integration_video_url_passthrough_pipeline() {
        let messages = vec![
            ChatMessage::user("Analyze [VIDEO:https://example.com/test.mp4]".to_string()),
        ];

        let prepared = prepare_messages_for_provider(&messages, &MultimodalConfig::default())
            .await
            .unwrap();

        assert!(prepared.contains_videos);
        assert!(!prepared.contains_images);
        assert_eq!(prepared.messages.len(), 1);

        let content = &prepared.messages[0].content;
        assert!(content.contains("[VIDEO:https://example.com/test.mp4]"));

        let (_, video_refs) = parse_video_markers(content);
        assert_eq!(video_refs.len(), 1);
        assert_eq!(video_refs[0], "https://example.com/test.mp4");

        let (_, image_refs) = parse_image_markers(content);
        assert!(image_refs.is_empty());
    }

    #[tokio::test]
    async fn integration_mixed_image_video_pipeline() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("test.png");
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
        )
        .unwrap();

        let messages = vec![ChatMessage::user(format!(
            "See [IMAGE:{}] and [VIDEO:https://example.com/v.mp4]",
            image_path.display()
        ))];

        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };

        let prepared = prepare_messages_for_provider(&messages, &config)
            .await
            .unwrap();

        assert!(prepared.contains_images);
        assert!(prepared.contains_videos);
        assert_eq!(prepared.messages.len(), 1);

        let content = &prepared.messages[0].content;
        let (_, image_refs) = parse_image_markers(content);
        assert_eq!(image_refs.len(), 1);
        assert!(image_refs[0].starts_with("data:image/png;base64,"));

        let (_, video_refs) = parse_video_markers(content);
        assert_eq!(video_refs.len(), 1);
        assert_eq!(video_refs[0], "https://example.com/v.mp4");
    }
    #[tokio::test]
    async fn integration_video_too_many_rejected() {
        let messages = vec![ChatMessage::user(
            "[VIDEO:https://a.mp4]\n[VIDEO:https://b.mp4]\n[VIDEO:https://c.mp4]".to_string(),
        )];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };
        let error = prepare_messages_for_provider(&messages, &config)
            .await
            .expect_err("should reject video count overflow");
        assert!(error.to_string().contains("multimodal video limit exceeded"));
    }
    #[tokio::test]
    async fn regression_image_only_pipeline_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let image_path = temp.path().join("regress.png");
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'],
        )
        .unwrap();
        let messages = vec![ChatMessage::user(format!(
            "Check [IMAGE:{}]",
            image_path.display()
        ))];
        let config = MultimodalConfig {
            max_images: 4,
            max_image_size_mb: 5,
            allow_remote_fetch: false,
            max_videos: 2,
            max_video_size_mb: 20,
        };
        let prepared = prepare_messages_for_provider(&messages, &config)
            .await
            .unwrap();
        assert!(prepared.contains_images);
        assert!(!prepared.contains_videos);
        assert_eq!(prepared.messages.len(), 1);
        let content = &prepared.messages[0].content;
        let (_, image_refs) = parse_image_markers(content);
        assert_eq!(image_refs.len(), 1);
        assert!(image_refs[0].starts_with("data:image/png;base64,"));
        let (_, video_refs) = parse_video_markers(content);
        assert!(video_refs.is_empty());
    }
}
