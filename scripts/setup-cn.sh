#!/usr/bin/env bash
# ZeroClaw China Deployment Setup
# Compiles with China-optimized features and downloads required model files.
# Usage: bash scripts/setup-cn.sh [--skip-build] [--skip-models] [--skip-browser] [--model-mirror URL]
set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
ZEROCLAW_HOME="${ZEROCLAW_HOME:-$HOME/.zeroclaw}"
MODELS_DIR="${ZEROCLAW_MODELS_DIR:-$ZEROCLAW_HOME/models}"
EMBEDDING_DIR="$MODELS_DIR/embeddinggemma-300m"
SENSEVOICE_DIR="$MODELS_DIR/sensevoice-small"
LIB_DIR="$ZEROCLAW_HOME/lib"

# HuggingFace mirror — hf-mirror.com is the standard China mirror.
# Override with HF_MIRROR env var or --model-mirror flag.
HF_MIRROR="${HF_MIRROR:-https://hf-mirror.com}"

# npm China mirror (npmmirror is the standard China mirror)
NPM_MIRROR="${NPM_MIRROR:-https://registry.npmmirror.com}"
# Features to compile for China deployment
CN_FEATURES="local-models,channel-lark,vpn,feishu-docs-sync"

SKIP_BUILD=false
SKIP_MODELS=false
SKIP_BROWSER=false

# ── Parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-build)  SKIP_BUILD=true; shift ;;
    --skip-models) SKIP_MODELS=true; shift ;;
    --model-mirror) HF_MIRROR="$2"; shift 2 ;;
    --no-transcription) CN_FEATURES="local-models,channel-lark,vpn"; shift ;;
    --skip-browser) SKIP_BROWSER=true; shift ;;
    -h|--help)
      echo "Usage: $0 [--skip-build] [--skip-models] [--skip-browser] [--no-transcription] [--model-mirror URL]"
      echo "  --skip-build        Skip cargo build (models only)"
      echo "  --skip-models       Skip model download (build only)"
      echo "  --skip-browser      Skip browser dependency installation"
      echo "  --no-transcription  Exclude local transcription models (Windows)"
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
  curl -fSL -C - --retry 5 --retry-delay 3 --connect-timeout 30 -o "$dest" "$url" \
    || err "Failed to download $url"
}

# Download a HuggingFace repo.  Prefers huggingface-cli (multi-threaded,
# resume-capable) and falls back to per-file curl.
hf_download() {
  local repo="$1" dest_dir="$2"
  shift 2
  local files=("$@")  # optional file list

  if command -v huggingface-cli &>/dev/null; then
    info "Using huggingface-cli for $repo (multi-threaded)..."
    local hf_args=(huggingface-cli download "$repo" --local-dir "$dest_dir")
    for f in "${files[@]}"; do hf_args+=("$f"); done
    if HF_ENDPOINT="$HF_MIRROR" "${hf_args[@]}"; then
      # Flatten subdirectory files to dest_dir (runtime expects flat layout)
      for f in "${files[@]}"; do
        local base; base="$(basename "$f")"
        if [[ "$f" != "$base" && -f "$dest_dir/$f" && ! -f "$dest_dir/$base" ]]; then
          mv "$dest_dir/$f" "$dest_dir/$base"
        fi
      done
      return 0
    fi
    info "huggingface-cli failed, falling back to curl..."
  fi

  # curl fallback: download each file individually
  mkdir -p "$dest_dir"
  for f in "${files[@]}"; do
    download "$HF_MIRROR/$repo/resolve/main/$f" "$dest_dir/$(basename "$f")"
  done
}

# ── Preflight ─────────────────────────────────────────────────────────────────
command -v curl >/dev/null || err "curl is required but not found"
if [[ "$SKIP_BUILD" == false ]]; then
  command -v cargo >/dev/null || err "cargo is required. Install Rust: https://rustup.rs"
  command -v protoc >/dev/null || { info "Installing protobuf-compiler..."; sudo apt-get install -y protobuf-compiler || err "Failed to install protoc"; }
fi

# ── Step 1: Build ─────────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" == false ]]; then
  if command -v zeroclaw &>/dev/null; then
    info "Removing existing zeroclaw binary before rebuild..."
    cargo uninstall zeroclaw 2>/dev/null || true
  fi
  cargo install --path . --force --features "$CN_FEATURES"
  info "Build complete: $(which zeroclaw 2>/dev/null || echo 'target/release/zeroclaw')"
else
  info "Skipping build (--skip-build)"
fi

# ── Step 1.5: Install clash-rs (VPN proxy backend) ────────────────────────────
# The vpn feature requires the `clash` binary in PATH.
# Download from GitHub releases since it's not on crates.io.
if [[ "$CN_FEATURES" == *"vpn"* ]]; then
  if command -v clash &>/dev/null || command -v clash-rs &>/dev/null; then
    info "clash-rs already installed, skipping"
  else
    info "Installing clash-rs from GitHub releases..."
    
    OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
    ARCH="$(uname -m)"
    
    case "$OS" in
      linux)
        case "$ARCH" in
          x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
          aarch64|arm64) TARGET="aarch64-unknown-linux-gnu" ;;
          armv7l) TARGET="armv7-unknown-linux-gnueabihf" ;;
          *) err "Unsupported architecture for clash-rs: $ARCH" ;;
        esac
        ;;
      darwin)
        case "$ARCH" in
          x86_64) TARGET="x86_64-apple-darwin" ;;
          aarch64|arm64) TARGET="aarch64-apple-darwin" ;;
          *) err "Unsupported architecture for clash-rs: $ARCH" ;;
        esac
        ;;
      msys*|cygwin*|mingw*)
        case "$ARCH" in
          x86_64) TARGET="x86_64-pc-windows-msvc.exe" ;;
          aarch64|arm64) TARGET="aarch64-pc-windows-msvc.exe" ;;
          *) err "Unsupported architecture for clash-rs: $ARCH" ;;
        esac
        ;;
      *) err "Unsupported OS for clash-rs: $OS" ;;
    esac
    
    CLASH_URL="https://github.com/Watfaq/clash-rs/releases/latest/download/clash-${TARGET}"
    CLASH_BIN_DIR="$HOME/.cargo/bin"
    mkdir -p "$CLASH_BIN_DIR"
    
    if [[ "$TARGET" == *".exe" ]]; then
      CLASH_BIN="$CLASH_BIN_DIR/clash-rs.exe"
    else
      CLASH_BIN="$CLASH_BIN_DIR/clash-rs"
    fi
    
    info "Downloading clash-rs for $TARGET..."
    curl -fSL --retry 3 --connect-timeout 30 -o "$CLASH_BIN" "$CLASH_URL" \
      || err "Failed to download clash-rs from $CLASH_URL"
    
    chmod +x "$CLASH_BIN"
    
    export PATH="$CLASH_BIN_DIR:$PATH"
    
    command -v clash-rs &>/dev/null && info "clash-rs installed: $(clash-rs --version 2>/dev/null || echo 'ok')" \
      || err "clash-rs installation failed"
  fi
fi

# ── Step 2: Download models ───────────────────────────────────────────────────
if [[ "$SKIP_MODELS" == false ]]; then
  info "Using HuggingFace mirror: $HF_MIRROR"
  info "Models directory: $MODELS_DIR"

  # ── 2a: EmbeddingGemma-300m Q8 ONNX ──
  if [[ -f "$EMBEDDING_DIR/model_quantized.onnx" && -f "$EMBEDDING_DIR/model_quantized.onnx_data" && -f "$EMBEDDING_DIR/tokenizer.json" ]]; then
    info "Embedding model already exists, skipping: $EMBEDDING_DIR"
  else
    hf_download "onnx-community/embeddinggemma-300m-ONNX" "$EMBEDDING_DIR" \
      "onnx/model_quantized.onnx" "onnx/model_quantized.onnx_data" "tokenizer.json"
    info "Embedding model ready: $EMBEDDING_DIR"
  fi

  # ── 2b: SenseVoice-Small (sherpa-onnx format) ──
  if [[ "$CN_FEATURES" == *"local-models"* ]]; then
    if [[ -f "$SENSEVOICE_DIR/model.onnx" && -f "$SENSEVOICE_DIR/tokens.txt" ]]; then
      info "SenseVoice model already exists, skipping: $SENSEVOICE_DIR"
    else
      hf_download "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17" "$SENSEVOICE_DIR" \
        "model.onnx" "tokens.txt"
      info "SenseVoice model ready: $SENSEVOICE_DIR"
    fi
  fi
else
  info "Skipping model download (--skip-models)"
fi

# ── Step 2.6: Install agent-browser and Playwright browsers ──────────────────
# agent-browser is the default browser automation backend; it depends on
# Playwright Chromium under the hood.  Use China npm/Playwright mirrors.
if [[ "$SKIP_BROWSER" == false ]]; then
  if ! command -v node &>/dev/null; then
    info "Node.js not found — installing via npmmirror..."
    NODE_MIRROR="https://registry.npmmirror.com/-/binary/node"
    NODE_VERSION="22.20.0"
    NODE_ARCH=$(uname -m)
    case "$NODE_ARCH" in aarch64) NODE_ARCH=arm64 ;; x86_64) NODE_ARCH=x64 ;; armv7l) NODE_ARCH=armv7l ;; *) err "Unsupported arch: $NODE_ARCH" ;; esac
    NODE_TAR="node-v${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz"
    info "Downloading Node.js v${NODE_VERSION} (${NODE_ARCH})..."
    curl -fSL --retry 3 --connect-timeout 30 -o "/tmp/$NODE_TAR" "$NODE_MIRROR/v${NODE_VERSION}/$NODE_TAR" \
      || err "Failed to download Node.js v${NODE_VERSION}"
    sudo tar -xJf "/tmp/$NODE_TAR" -C /usr/local --strip-components=1
    rm -f "/tmp/$NODE_TAR"
    command -v node &>/dev/null || err "Node.js installation failed"
  fi

  if command -v agent-browser &>/dev/null; then
    info "agent-browser already installed, skipping"
  else
    info "Installing agent-browser via China npm mirror ($NPM_MIRROR)..."
    npm install -g agent-browser --registry="$NPM_MIRROR" \
      || err "Failed to install agent-browser"
  fi

  # Check if Playwright Chromium is already installed via INSTALLATION_COMPLETE marker.
  # Playwright stores browsers under ~/.cache/ms-playwright/chromium-*/.
  PW_CHROMIUM_INSTALLED=false
  for d in "$HOME/.cache/ms-playwright"/chromium-*/; do
    if [[ -f "${d}INSTALLATION_COMPLETE" ]]; then
      PW_CHROMIUM_INSTALLED=true
      break
    fi
  done
  if [[ "$PW_CHROMIUM_INSTALLED" == true ]]; then
    info "Playwright Chromium already installed, skipping"
  else
    info "Installing Playwright Chromium via China mirror..."
    # Use Playwright's own install command with PLAYWRIGHT_DOWNLOAD_HOST for China mirror.
    # This ensures correct revision, directory structure, and INSTALLATION_COMPLETE marker.
    PLAYWRIGHT_DOWNLOAD_HOST="https://npmmirror.com/mirrors/playwright" \
      npx --registry="$NPM_MIRROR" playwright install chromium \
      || err "Failed to install Playwright Chromium (tried npmmirror mirror)"
  fi

  # Install Playwright system dependencies (fonts, libs) on Linux
  if [[ "$(uname)" == "Linux" ]]; then
    npx --registry="$NPM_MIRROR" playwright install-deps chromium \
      || info "WARNING: Failed to install Playwright system deps — browser may not work headless"
  fi

  info "Browser dependencies ready (agent-browser + Playwright Chromium)"
else
  info "Skipping browser dependencies (--skip-browser)"
fi

# ── Step 3: Write config via zeroclaw onboard ────────────────────────────────
info "Generating base config via zeroclaw onboard..."
zeroclaw onboard --provider qwen --memory lancedb --force

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
if [[ "$CN_FEATURES" == *"local-models"* ]]; then
  ensure_toml_key transcription provider '"local"'
  ensure_toml_key transcription model '"'"$SENSEVOICE_DIR"'"'
fi
if [[ "$SKIP_BROWSER" == false ]]; then
  ensure_toml_key browser enabled true
  ensure_toml_key browser backend '"agent_browser"'
fi
info "Config patched with local embedding/transcription settings"

info "To set your API key, run: zeroclaw onboard --provider qwen --api-key sk-YOUR_DASHSCOPE_KEY"
