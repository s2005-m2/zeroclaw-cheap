#!/usr/bin/env bash
# ZeroClaw China Deployment Setup
# Compiles with China-optimized features and downloads required model files.
# Usage: bash scripts/setup-cn.sh [--skip-build] [--skip-models] [--model-mirror URL]
set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
ZEROCLAW_HOME="${ZEROCLAW_HOME:-$HOME/.zeroclaw}"
MODELS_DIR="${ZEROCLAW_MODELS_DIR:-$ZEROCLAW_HOME/models}"
EMBEDDING_DIR="$MODELS_DIR/embeddinggemma-300m"
SENSEVOICE_DIR="$MODELS_DIR/sensevoice-small"

# HuggingFace mirror (hf-mirror.com is the standard China mirror)
HF_MIRROR="${HF_MIRROR:-https://hf-mirror.com}"

# Features to compile for China deployment
CN_FEATURES="local-embedding,memory-lancedb,local-transcription,channel-lark"

SKIP_BUILD=false
SKIP_MODELS=false

# ── Parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-build)  SKIP_BUILD=true; shift ;;
    --skip-models) SKIP_MODELS=true; shift ;;
    --model-mirror) HF_MIRROR="$2"; shift 2 ;;
    --no-transcription) CN_FEATURES="local-embedding,memory-lancedb,channel-lark"; shift ;;
    -h|--help)
      echo "Usage: $0 [--skip-build] [--skip-models] [--no-transcription] [--model-mirror URL]"
      echo "  --skip-build        Skip cargo build (models only)"
      echo "  --skip-models       Skip model download (build only)"
      echo "  --no-transcription  Exclude local-transcription feature (Windows)"
      echo "  --model-mirror URL  HuggingFace mirror (default: https://hf-mirror.com)"
      exit 0 ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

info() { echo "[setup-cn] $*"; }
err()  { echo "[setup-cn] ERROR: $*" >&2; exit 1; }

download() {
  local url="$1" dest="$2"
  if [[ -f "$dest" ]]; then
    info "Already exists: $dest"
    return 0
  fi
  info "Downloading: $url"
  curl -fSL --retry 3 --connect-timeout 30 -o "$dest" "$url" \
    || err "Failed to download $url"
}

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v curl >/dev/null || err "curl is required but not found"
if [[ "$SKIP_BUILD" == false ]]; then
  command -v cargo >/dev/null || err "cargo is required. Install Rust: https://rustup.rs"
fi

# ── Step 1: Build ─────────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" == false ]]; then
  info "Building ZeroClaw with China features: $CN_FEATURES"
  cargo build --release --features "$CN_FEATURES"
  info "Installing to ~/.cargo/bin/zeroclaw"
  cargo install --path . --force --features "$CN_FEATURES"
  info "Build complete: $(which zeroclaw 2>/dev/null || echo 'target/release/zeroclaw')"
else
  info "Skipping build (--skip-build)"
fi

# ── Step 2: Download models ───────────────────────────────────────────────────
if [[ "$SKIP_MODELS" == false ]]; then
  info "Using HuggingFace mirror: $HF_MIRROR"
  info "Models directory: $MODELS_DIR"

  # ── 2a: EmbeddingGemma-300m Q8 ONNX ──
  EMBED_REPO="onnx-community/embeddinggemma-300m-ONNX"
  EMBED_BASE="$HF_MIRROR/$EMBED_REPO/resolve/main/onnx"
  mkdir -p "$EMBEDDING_DIR"
  download "$EMBED_BASE/model_quantized.onnx"      "$EMBEDDING_DIR/model_quantized.onnx"
  download "$EMBED_BASE/model_quantized.onnx_data" "$EMBEDDING_DIR/model_quantized.onnx_data"
  download "$HF_MIRROR/$EMBED_REPO/resolve/main/tokenizer.json" "$EMBEDDING_DIR/tokenizer.json"
  info "Embedding model ready: $EMBEDDING_DIR"

  # ── 2b: SenseVoice-Small (sherpa-onnx format) ──
  if [[ "$CN_FEATURES" == *"local-transcription"* ]]; then
    SV_REPO="csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
    SV_BASE="$HF_MIRROR/$SV_REPO/resolve/main"
    mkdir -p "$SENSEVOICE_DIR"
    download "$SV_BASE/model.onnx"  "$SENSEVOICE_DIR/model.onnx"
    download "$SV_BASE/tokens.txt" "$SENSEVOICE_DIR/tokens.txt"
    info "SenseVoice model ready: $SENSEVOICE_DIR"
  fi
else
  info "Skipping model download (--skip-models)"
fi

# ── Step 3: Write config via zeroclaw onboard ────────────────────────────────
info "Generating base config via zeroclaw onboard..."
zeroclaw onboard --provider qwen --memory lancedb --compact-context --force

# Post-patch settings not covered by onboard flags
CONFIG_FILE="$ZEROCLAW_HOME/config.toml"
ensure_toml_key() {
  local section="$1" key="$2" value="$3"
  if grep -q "^\[$section\]" "$CONFIG_FILE"; then
    if ! grep -q "^$key\s*=" "$CONFIG_FILE"; then
      sed -i.bak "/^\[$section\]/a $key = $value" "$CONFIG_FILE" && rm -f "${CONFIG_FILE}.bak"
    fi
  else
    printf '\n[%s]\n%s = %s\n' "$section" "$key" "$value" >> "$CONFIG_FILE"
  fi
}
ensure_toml_key memory embedding_provider '"local"'
ensure_toml_key memory embedding_model '"'"$EMBEDDING_DIR"'"'
ensure_toml_key memory embedding_dims 768
if [[ "$CN_FEATURES" == *"local-transcription"* ]]; then
  ensure_toml_key transcription provider '"local"'
  ensure_toml_key transcription model '"'"$SENSEVOICE_DIR"'"'
fi
info "Config patched with local embedding/transcription settings"

info "To set your API key, run: zeroclaw onboard --provider qwen --api-key sk-YOUR_DASHSCOPE_KEY"
