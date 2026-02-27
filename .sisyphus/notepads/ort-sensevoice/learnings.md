## Learnings

### 2026-02-27: ort crate WSL2 build validation

- **Rust 1.93.1** installed on WSL2 Ubuntu 22.04 (glibc 2.35)
- **Static linking FAILS**: ort-sys prebuilt `libonnxruntime.a` requires glibc 2.38+ (`__isoc23_strtoll` etc.)
- **Dynamic linking WORKS**: Add `load-dynamic` feature to skip static linking, load `.so` at runtime
- Working features: `["std", "download-binaries", "tls-rustls", "load-dynamic", "copy-dylibs"]`
- Runtime needs `libonnxruntime.so` 1.23.0 via `LD_LIBRARY_PATH` or `ORT_DYLIB_PATH` env var
- Binary size with load-dynamic: ~590KB (shared lib is ~22MB separate)
- Build time: ~16s release on WSL2 (20 cores)
- ort resolves to 2.0.0-rc.11 even when specifying 2.0.0-rc.9 (semver compatible)

## Decisions
### 2026-02-27: Build approach decision
- **Native WSL2 build confirmed** as the path forward (not cross-compile from Windows)
- Use `load-dynamic` feature instead of default static linking to avoid glibc mismatch
- Ship `libonnxruntime.so.1.23.0` (~22MB) alongside the binary at deployment time
- Set `ORT_DYLIB_PATH` env var at runtime to point to the shared library

## Issues
### 2026-02-27: glibc mismatch with ort-sys static lib
- ort-sys 2.0.0-rc.11 prebuilt `libonnxruntime.a` compiled against glibc 2.38+
- Ubuntu 22.04 ships glibc 2.35 — missing `__isoc23_strtoll`, `__isoc23_strtol`, `__isoc23_strtoull`
- Workaround: use `load-dynamic` feature to avoid static linking entirely
- Alternative: upgrade to Ubuntu 24.04+ (glibc 2.39) for static linking support

## Problems

### 2026-02-27: SenseVoice Fbank + pipeline research (Task 2)

- **sensevoice-rs** is by `darkautism` (not Patchethium) — crate `sensevoice-rs` v0.1.7 on crates.io
- Uses `kaldi-fbank-rust-kautism` (C++ FFI to kaldi-native-fbank) — NOT pure Rust
- Fbank params confirmed: 16kHz, 25ms window (400 samples), 10ms shift (160 samples), 80 mel bins, Hamming window, dither=0.0, preemph=0.97, snip_edges=true
- FFT size = 512 (next power of 2 above 400), mel range 20-8000 Hz
- LFR: window=7, shift=6, output=560-dim. **Two different implementations exist:**
  - sensevoice-rs: left-pads with 3 copies of first frame, ceil(T/6) output frames
  - sherpa-onnx: no padding, (T-7)/6+1 output frames (simpler, fewer frames)
  - **Decision: use sherpa-onnx LFR style** since we use sherpa-onnx-exported ONNX model
- CMVN: `(feature + neg_mean) * inv_stddev` — vectors are 560-dim, from ONNX model metadata
- `normalize_samples = 0` means audio stays in int16 range (cast i16→f32, no /32768 normalization)
- CTC decode: argmax → remove blank_id=0 → remove consecutive duplicates → token lookup
- First 4 output tokens are prompt tokens (lang, emotion, event, punct) — skip them for text
- tokens.txt format: `symbol_text  integer_id` per line, 25055 tokens total
- **Pure Rust path:** only need `rustfft` (MIT/Apache-2.0) for FFT — no C++ FFI needed
- Avoid: kaldi-fbank-rust (C++ FFI), sentencepiece (C++ FFI), candle (unnecessary for ONNX path)

### 2026-02-27: Task 3+4 — ort_transcription.rs implementation

- **ort 2.0.0-rc.11 metadata API**: `metadata.custom(key)` takes a `&str` key and returns `Option<String>` — NOT a HashMap. The context7 docs showed iteration but the actual API is per-key lookup.
- **rustfft 6.4.1**: Uses `FftPlanner::new()` + `plan_fft_forward(512)` + `process(&mut buf)` with `Complex<f32>` buffer. Works well for pure-Rust Fbank.
- **Clippy strictness**: Project uses `-D warnings` which catches `cast_sign_loss`, `cast_possible_truncation`, `implicit_hasher`, `needless_range_loop`, `cast_lossless`, `needless_borrow`. Use `unsigned_abs()` for i32→usize, `i64::from()` for i32→i64, `i32::try_from()` for usize→i32.
- **Windows PDB limit**: Debug builds on Windows hit `LNK1318: PDB error LIMIT (12)` — use `--release` for test runs. This is a pre-existing project issue.
- **Pre-existing clippy failures**: 25+ clippy errors exist in other files (hook_write.rs, skill_manage.rs, cron/scheduler.rs, etc.) — not introduced by this task.
- **Module registration**: File created but NOT registered in `mod.rs` — Task 5 will add `#[cfg(feature = "local-embedding")] pub mod ort_transcription;`.
- **Feature gating**: `rustfft` added as optional dep under `local-embedding` feature (temporary — Task 6 renames to `local-models`).
- **All 11 unit tests pass** in release mode: fbank dims, LFR dims, CTC decode, tokens parsing, mel filterbank shape, CMVN, fbank with sine wave input.
