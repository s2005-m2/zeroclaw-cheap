# Task 2: SenseVoice Fbank + LFR + CMVN + CTC Decode Pipeline Specification

**Date:** 2026-02-27
**Sources:**
- `darkautism/sensevoice-rs` (v0.1.7) — Rust implementation
- `k2-fsa/sherpa-onnx` — C++ reference implementation
- `darkautism/kaldi-fbank-rust` — Kaldi fbank Rust bindings (C++ FFI)

---

## 1. Pipeline Overview

```
PCM i16 audio
  → float32 conversion (sample as f32, no normalization to [-1,1])
  → 80-dim log-mel Fbank (kaldi-native-fbank, 25ms window, 10ms shift)
  → LFR: 7-frame concat with 6-frame shift → 560-dim
  → CMVN: (feature + neg_mean) * inv_stddev
  → ONNX model input: [B, T_lfr, 560]
  → CTC logits [B, T_out, vocab_size]
  → greedy decode: argmax → remove blanks → dedup → token lookup
```

---

## 2. Fbank Feature Extraction (Exact Parameters)

### 2.1 Parameters Used by sensevoice-rs

From `wavfrontend.rs` `compute_fbank_features()`:

| Parameter | Value | Notes |
|-----------|-------|-------|
| **Sample rate** | 16000 Hz | `samp_freq: 16000.0` |
| **Frame length** | 25.0 ms (400 samples) | `frame_length_ms: 25.0` |
| **Frame shift** | 10.0 ms (160 samples) | `frame_shift_ms: 10.0` |
| **Window function** | `"hamming"` | Overridden from default `"povey"` |
| **Num mel bins** | 80 | `num_bins: 80` |
| **Dither** | 0.0 | Explicitly disabled |
| **snip_edges** | true | Kaldi default |
| **energy_floor** | 0.0 | |
| **use_log_fbank** | true | Log mel filterbank (kaldi default) |
| **use_power** | true | Power spectrum (kaldi default) |

### 2.2 Kaldi-native-fbank Default Parameters (from kaldi-fbank-rust)

These are the C++ library defaults; sensevoice-rs overrides some:

| Parameter | Library Default | SenseVoice Override |
|-----------|----------------|---------------------|
| `samp_freq` | 16000.0 | 16000.0 (same) |
| `frame_shift_ms` | 10.0 | 10.0 (same) |
| `frame_length_ms` | 25.0 | 25.0 (same) |
| `dither` | 0.00003 | **0.0** (disabled) |
| `preemph_coeff` | 0.97 | 0.97 (default, not overridden) |
| `remove_dc_offset` | true | true (default) |
| `window_type` | `"povey"` | **`"hamming"`** |
| `round_to_power_of_two` | true | true (default) |
| `blackman_coeff` | 0.42 | 0.42 (default) |
| `snip_edges` | true | true (same) |
| `num_bins` (mel) | 25 | **80** |
| `low_freq` | 20.0 | 20.0 (default) |
| `high_freq` | 0.0 | 0.0 (default = Nyquist = 8000 Hz) |
| `htk_mode` | false | false (default) |
| `is_librosa` | false | false (default) |
| `norm` | `"slaney"` | `"slaney"` (default) |
| `use_energy` | false | false (default) |
| `raw_energy` | true | true (default) |
| `htk_compat` | false | false (default) |
| `use_log_fbank` | true | true (default) |
| `use_power` | true | true (default) |

### 2.3 Sherpa-onnx Cross-Validation (InitFeatConfig)

From `offline-recognizer-sense-voice-impl.h`:
```cpp
config_.feat_config.window_type = "hamming";
config_.feat_config.high_freq = 0;
config_.feat_config.snip_edges = true;
config_.feat_config.normalize_samples = meta_data.normalize_samples; // 0 = false
```

**CRITICAL:** `normalize_samples = 0` means samples are **NOT** normalized to [-1, 1].
Instead, they are scaled to [-32768, 32767] range before feature extraction.
In sherpa-onnx: `buf[i] = waveform[i] * 32768` when normalize_samples is false.
In sensevoice-rs: input is `&[i16]` cast directly to `f32` — same effect.

### 2.4 FFT Size

With `round_to_power_of_two = true` and `frame_length_ms = 25.0` at 16kHz:
- Window length = 400 samples
- **FFT size = 512** (next power of 2 above 400)

### 2.5 Mel Frequency Range

- `low_freq = 20.0 Hz`
- `high_freq = 0.0` → interpreted as Nyquist frequency = **8000 Hz** (at 16kHz sample rate)
- Mel scale: Kaldi default (not HTK mode), with `"slaney"` normalization

### 2.6 Output

- Shape: `[num_frames, 80]` where `num_frames = floor((num_samples - 400) / 160) + 1` (with snip_edges=true)
- Values: log mel filterbank energies (float32)

---

## 3. LFR (Low Frame Rate) Concatenation

### 3.1 Parameters

| Parameter | Value | Source |
|-----------|-------|--------|
| `lfr_m` (window size) | 7 | Model metadata `lfr_window_size`, hardcoded in sensevoice-rs |
| `lfr_n` (window shift) | 6 | Model metadata `lfr_window_shift`, hardcoded in sensevoice-rs |
| Input dimension | 80 (mel bins) | From Fbank output |
| Output dimension | **560** (80 × 7) | Concatenation of 7 frames |

### 3.2 Algorithm (sensevoice-rs `apply_lfr`)

```
Input: fbank[T, 80]
Output: lfr[T_lfr, 560]

1. Compute output frames: T_lfr = ceil(T / lfr_n)
2. Left-pad with (lfr_m - 1) / 2 = 3 copies of the first frame
3. For each output frame i (0..T_lfr):
   - start = i * lfr_n
   - Concatenate lfr_m=7 consecutive frames from padded_fbank[start..start+7]
   - If fewer than 7 frames remain, repeat the last frame to fill
4. Result shape: [T_lfr, 560]
```

### 3.3 Sherpa-onnx LFR Algorithm (different!)

**IMPORTANT DIFFERENCE:** sherpa-onnx uses a simpler sliding window without padding:

```cpp
out_num_frames = (in_num_frames - lfr_window_size) / lfr_window_shift + 1;
// No padding. Simply copies lfr_window_size * feat_dim contiguous floats.
// Slides by lfr_window_shift * feat_dim each step.
```

This means sherpa-onnx produces **fewer output frames** than sensevoice-rs for the same input.
For our implementation, we should follow the **sherpa-onnx approach** since we use the ONNX model
exported by sherpa-onnx tooling.

### 3.4 Decision: Which LFR to Use

- **For ONNX model path (our target):** Use sherpa-onnx style (no padding, simple slide)
- **For Candle .pt path:** sensevoice-rs style (with left-padding) matches the Python FunASR frontend
- The ONNX model from sherpa-onnx was exported expecting sherpa-onnx's LFR behavior

---

## 4. CMVN (Cepstral Mean and Variance Normalization)

### 4.1 Data Source

Two approaches exist for loading CMVN data:

**Approach A: ONNX model metadata (sherpa-onnx)**
- `neg_mean`: Vec<f32> of length 560, stored in ONNX model metadata key `"neg_mean"`
- `inv_stddev`: Vec<f32> of length 560, stored in ONNX model metadata key `"inv_stddev"`
- These are read at model load time via `Ort::ModelMetadata`

**Approach B: External am.mvn file (sensevoice-rs)**
- File format: Kaldi-style CMVN file with `<AddShift>` and `<Rescale>` sections
- `<AddShift>` section contains means (parsed from `<LearnRateCoef>` line)
- `<Rescale>` section contains vars (parsed from `<LearnRateCoef>` line)
- File: `am.mvn` from HuggingFace `FunAudioLLM/SenseVoiceSmall`

### 4.2 Formula

```
normalized_feature = (feature + neg_mean) * inv_stddev
```

Where:
- `feature`: one 560-dim LFR frame
- `neg_mean`: 560-dim vector (negative of the mean, so addition = subtraction of mean)
- `inv_stddev`: 560-dim vector (inverse of standard deviation, so multiplication = division)

Applied element-wise, broadcast across all frames.

### 4.3 Implementation (sherpa-onnx, Eigen)

```cpp
// From offline-recognizer-sense-voice-impl.h
mat.array() = (mat.array().rowwise() + neg_mean_vec.array()).rowwise() * inv_stddev_vec.array();
```

### 4.4 Implementation (sensevoice-rs, ndarray)

```rust
// From wavfrontend.rs apply_cmvn()
(feats + &means_expanded) * &vars_expanded
```

### 4.5 Decision for ZeroClaw

- **Use ONNX model metadata** for neg_mean/inv_stddev (Approach A)
- This avoids shipping a separate `am.mvn` file
- Parse from model metadata at session init time

---

## 5. ONNX Model Input/Output Specification

### 5.1 Model Inputs (4 inputs)

| Input Name | Shape | Dtype | Description |
|------------|-------|-------|-------------|
| `speech` / `features` | `[B, T, 560]` | float32 | CMVN-normalized LFR features |
| `speech_lengths` / `features_length` | `[B]` | int32 | Number of valid LFR frames per batch item |
| `language` | `[B]` | int32 | Language ID from model metadata |
| `text_norm` | `[B]` | int32 | Text normalization mode ID |

### 5.2 Model Output

| Output | Shape | Dtype | Description |
|--------|-------|-------|-------------|
| `logits` | `[B, T_out, vocab_size]` | float32 | CTC logits per frame |

Where `T_out = T + 4` (4 prompt tokens prepended internally by the model).

### 5.3 Language IDs (from model metadata)

| Language | Metadata Key | Default ID |
|----------|-------------|------------|
| auto | `lang_auto` | 0 |
| zh (Mandarin) | `lang_zh` | 3 |
| en (English) | `lang_en` | 4 |
| yue (Cantonese) | `lang_yue` | 7 |
| ja (Japanese) | `lang_ja` | 11 |
| ko (Korean) | `lang_ko` | 12 |

### 5.4 Text Normalization IDs (from model metadata)

| Mode | Metadata Key | Default ID |
|------|-------------|------------|
| With ITN (inverse text normalization) | `with_itn` | 14 |
| Without ITN | `without_itn` | 15 |

### 5.5 Other Model Metadata Keys

| Key | Type | Description |
|-----|------|-------------|
| `vocab_size` | int32 | 25055 (default) |
| `blank_id` | int32 | 0 (default, CTC blank token) |
| `lfr_window_size` | int32 | 7 |
| `lfr_window_shift` | int32 | 6 |
| `normalize_samples` | int32 | 0 (do NOT normalize to [-1,1]) |
| `neg_mean` | Vec<f32> | 560-dim CMVN negative mean |
| `inv_stddev` | Vec<f32> | 560-dim CMVN inverse stddev |
| `comment` | string | Contains "Nano" for FunASR Nano variant |

---

## 6. CTC Greedy Decoding

### 6.1 Algorithm (sherpa-onnx reference)

```
Input: logits[B, T_out, vocab_size] float32
       logits_length[B] int64 — valid frame count per batch item

For each batch item b:
  prev_id = -1
  tokens = []
  For each frame t in 0..logits_length[b]:
    y = argmax(logits[b, t, :])  // index of max value across vocab dim
    if y != blank_id AND y != prev_id:
      tokens.append(y)
    prev_id = y
```

Key points:
- **blank_id = 0** (from model metadata)
- Removes both blank tokens AND consecutive duplicates in one pass
- Output is a list of token IDs
### 6.2 SenseVoice Output Format
The first 4 tokens of the CTC output are special prompt tokens (skipped in sherpa-onnx):
- Token 0: language tag (e.g., `<|zh|>`, `<|en|>`)
- Token 1: emotion tag (e.g., `<|HAPPY|>`, `<|NEUTRAL|>`)
- Token 2: event tag (e.g., `<|BGM|>`, `<|SPEECH|>`)
- Token 3: punctuation normalization tag (e.g., `<|woitn|>`, `<|with|>`)
- Tokens 4+: actual transcription content
In sherpa-onnx, `ConvertSenseVoiceResult` starts from index 4 (skipping prompt tokens).
In sensevoice-rs, the full string is parsed with regex:
```
^<\|(.*?)\|><\|(.*?)\|><\|(.*?)\|><\|(.*?)\|>(.*)$
```
### 6.3 sensevoice-rs CTC Decode (ids_to_text)
```rust
// From lib.rs ids_to_text()
let mut unique_ids = Vec::new();
let mut prev_id = None;
for &id in token_ids.iter() {
    if Some(id) != prev_id && id != 0 {  // 0 = blank_id
        unique_ids.push(id as u32);
        prev_id = Some(id);
    } else if Some(id) != prev_id {
        prev_id = Some(id);
    }
}
// Decode via SentencePiece
let decoded_text = spp.decode_piece_ids(&unique_ids)?;
```
---
## 7. tokens.txt Format
### 7.1 File Format (sherpa-onnx SymbolTable)
Each line contains two fields separated by space(s):
```
symbol_text  integer_id
```
Example:
```
<blank> 0
<sos> 1
<eos> 2
▁the 42
hello 1337
<|zh|> 25000
<|en|> 25001
<|HAPPY|> 25010
```
### 7.2 Special Tokens
| Token | ID | Purpose |
|-------|-----|---------|
| `<blank>` | 0 | CTC blank token (blank_id) |
| `<|zh|>` | varies | Chinese language tag |
| `<|en|>` | varies | English language tag |
| `<|ja|>` | varies | Japanese language tag |
| `<|ko|>` | varies | Korean language tag |
| `<|yue|>` | varies | Cantonese language tag |
| `<|HAPPY|>` | varies | Emotion: happy |
| `<|SAD|>` | varies | Emotion: sad |
| `<|ANGRY|>` | varies | Emotion: angry |
| `<|NEUTRAL|>` | varies | Emotion: neutral |
| `<|BGM|>` | varies | Event: background music |
| `<|SPEECH|>` | varies | Event: speech |
| `<|with|>` | varies | With punctuation normalization |
| `<|woitn|>` | varies | Without punctuation normalization |
### 7.3 Tokenizer Choice
- **sherpa-onnx:** Uses `tokens.txt` (plain text symbol table)
- **sensevoice-rs:** Uses SentencePiece BPE model (`chn_jpn_yue_eng_ko_spectok.bpe.model`)
- **For ZeroClaw:** Use `tokens.txt` from sherpa-onnx export (simpler, no SentencePiece dependency)
- Token count: **25055** (vocab_size from model metadata)
---
## 8. Required Rust Crates
### 8.1 For Pure-Rust Fbank (our approach — NO C++ FFI)
We will NOT use `kaldi-fbank-rust` (C++ FFI bindings). Instead, implement Fbank in pure Rust.
| Crate | Purpose | License | Version |
|-------|---------|---------|---------|
| `rustfft` | FFT computation (real-to-complex) | MIT/Apache-2.0 | latest |
| `ndarray` | N-dimensional array operations | MIT/Apache-2.0 | 0.16+ |
| (std only) | Hamming window, mel filterbank, pre-emphasis | N/A | N/A |
### 8.2 For ONNX Inference
| Crate | Purpose | License | Version |
|-------|---------|---------|---------|
| `ort` | ONNX Runtime bindings | MIT/Apache-2.0 | 2.0.0-rc.9+ |
### 8.3 For Token Decoding
| Crate | Purpose | License | Version |
|-------|---------|---------|---------|
| (std only) | tokens.txt parser, CTC greedy decode | N/A | N/A |
### 8.4 Crates NOT Needed (avoided dependencies)
| Crate | Why NOT | Alternative |
|-------|---------|-------------|
| `kaldi-fbank-rust` | C++ FFI, build complexity | Pure Rust Fbank |
| `sentencepiece` | C++ FFI, only needed for .bpe.model | tokens.txt lookup |
| `candle` / `candle-nn` | Only for .pt model path | ONNX via ort |
| `hf-hub` | Model download at runtime | Ship model files |
| `voice_activity_detector` | VAD is separate concern | Existing sherpa-rs VAD |
---
## 9. Pure-Rust Fbank Implementation Spec
### 9.1 Algorithm Steps
```
1. Pre-emphasis: y[n] = x[n] - 0.97 * x[n-1]
2. Framing: 400-sample windows, 160-sample hop, snip_edges=true
3. Hamming window: w[n] = 0.54 - 0.46 * cos(2πn / (N-1))
4. Zero-pad frame to 512 samples (next power of 2)
5. FFT: 512-point real FFT → 257 complex bins
6. Power spectrum: |X[k]|^2 for k=0..256
7. Mel filterbank: 80 triangular filters, 20 Hz to 8000 Hz
8. Log: output[m] = log(max(energy_floor, dot(power_spec, mel_filter[m])))
```
### 9.2 Framing Details
```
num_samples = len(waveform)
frame_length = 400  # 25ms * 16000
frame_shift = 160   # 10ms * 16000
num_frames = (num_samples - frame_length) / frame_shift + 1  # snip_edges=true
// With snip_edges=true, last frame must fit entirely within signal
```
### 9.3 DC Offset Removal
```
// remove_dc_offset = true (kaldi default)
mean = sum(frame) / frame_length
frame[n] -= mean  // for each sample in frame
```
### 9.4 Mel Filterbank Construction
```
// Kaldi-style mel filterbank (not HTK mode)
// Mel scale: mel(f) = 1127 * ln(1 + f/700)
// 80 triangular filters spanning 20 Hz to 8000 Hz
// Slaney normalization: each filter area normalized
// Power spectrum bins: 257 (from 512-point FFT)
// Frequency resolution: 16000/512 = 31.25 Hz per bin
```
### 9.5 Processing Order (Critical)
```
1. Extract frame from signal (with frame_shift hop)
2. Remove DC offset (subtract mean)
3. Apply pre-emphasis (coeff=0.97) to frame
4. Apply Hamming window
5. Zero-pad to 512
6. Compute FFT
7. Compute power spectrum
8. Apply mel filterbank
9. Take log
```
Note: dither=0.0 means NO dithering is applied.
---
## 10. Sample Rate and Input Format
### 10.1 Audio Input Contract
- **Expected sample rate:** 16000 Hz (16 kHz)
- **Accepted formats:** 8000 Hz and 16000 Hz (sensevoice-rs validates this)
- **Sample format:** 16-bit signed integer (i16)
- **Channels:** mono (single channel)
- **Conversion:** `i16` → `f32` by simple cast (`sample as f32`), NOT normalized to [-1, 1]
- This matches `normalize_samples = 0` in model metadata
### 10.2 Why NOT Normalize
sherpa-onnx explicitly handles this:
```cpp
// When normalize_samples is false:
buf[i] = waveform[i] * 32768;  // scale float [-1,1] to int16 range
```
sensevoice-rs takes `&[i16]` directly and casts to f32, achieving the same effect.
---
## 11. End-to-End Pipeline Summary (ZeroClaw Target)
```
1. Load ONNX model via ort (load-dynamic)
2. Extract metadata: neg_mean, inv_stddev, blank_id, lang IDs, itn IDs, lfr params
3. Load tokens.txt into HashMap<i32, String>
4. On audio input:
   a. Receive PCM i16 mono 16kHz samples
   b. Cast to f32 (no normalization)
   c. Compute 80-dim log-mel Fbank (pure Rust: rustfft + custom mel)
   d. Apply LFR: slide window_size=7, shift=6 → 560-dim frames (sherpa-onnx style)
   e. Apply CMVN: (frame + neg_mean) * inv_stddev
   f. Build input tensors:
      - features: [1, T_lfr, 560] f32
      - features_length: [1] i32 = T_lfr
      - language: [1] i32 = lang_id (0=auto, 4=en, 3=zh, etc.)
      - text_norm: [1] i32 = 15 (without_itn) or 14 (with_itn)
   g. Run ONNX inference → logits [1, T_lfr+4, 25055]
   h. CTC greedy decode: argmax → skip blanks → dedup → token IDs
   i. Skip first 4 tokens (prompt: lang, emo, event, punct)
   j. Map remaining token IDs → text via tokens.txt
```
---
## 12. Key Differences: sensevoice-rs vs sherpa-onnx
| Aspect | sensevoice-rs | sherpa-onnx | ZeroClaw choice |
|--------|--------------|-------------|-----------------|
| Fbank engine | kaldi-fbank-rust (C++ FFI) | kaldi-native-fbank (C++) | Pure Rust |
| LFR | Left-padded, ceil(T/n) frames | No padding, (T-m)/n+1 frames | sherpa-onnx style |
| CMVN source | am.mvn file | ONNX model metadata | ONNX metadata |
| Tokenizer | SentencePiece .bpe.model | tokens.txt symbol table | tokens.txt |
| Model format | .pt (Candle) or RKNN | ONNX | ONNX |
| Inference | Candle / RKNN | ONNX Runtime | ort crate |
