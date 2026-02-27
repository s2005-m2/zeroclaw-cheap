// Gated by #[cfg(feature = "local-embedding")] in mod.rs (Task 5)
//! SenseVoice speech-to-text inference pipeline using pure-Rust Fbank + ort ONNX Runtime.
//!
//! Pipeline: PCM i16 → f32 → 80-dim log-mel Fbank → LFR (560-dim) → CMVN → ONNX → CTC decode → text

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use ort::session::Session;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

// ── Constants ────────────────────────────────────────────────────────────────

const SAMPLE_RATE: f32 = 16000.0;
const FRAME_LENGTH_SAMPLES: usize = 400; // 25ms at 16kHz
const FRAME_SHIFT_SAMPLES: usize = 160; // 10ms at 16kHz
const FFT_SIZE: usize = 512; // next power of 2 above 400
const NUM_MEL_BINS: usize = 80;
const LOW_FREQ: f32 = 20.0;
const HIGH_FREQ: f32 = 8000.0; // Nyquist at 16kHz (high_freq=0 → Nyquist)
const PRE_EMPHASIS: f32 = 0.97;
const ENERGY_FLOOR: f32 = 1e-10;
const NUM_FFT_BINS: usize = FFT_SIZE / 2 + 1; // 257

// ── SenseVoiceMetadata ───────────────────────────────────────────────────────

/// Metadata extracted from the ONNX model's custom metadata map.
pub struct SenseVoiceMetadata {
    pub vocab_size: i32,
    pub blank_id: i32,
    pub lfr_window_size: i32,
    pub lfr_window_shift: i32,
    pub normalize_samples: bool,
    pub neg_mean: Vec<f32>,
    pub inv_stddev: Vec<f32>,
    pub lang_auto: i32,
    pub lang_zh: i32,
    pub lang_en: i32,
    pub lang_ja: i32,
    pub lang_ko: i32,
    pub lang_yue: i32,
    pub with_itn: i32,
    pub without_itn: i32,
}

// ── Metadata reader ──────────────────────────────────────────────────────────

/// Extract SenseVoice pipeline metadata from an ONNX model session.
pub fn read_model_metadata(session: &Session) -> Result<SenseVoiceMetadata> {
    let metadata = session
        .metadata()
        .context("failed to read model metadata")?;
    let get_str = |key: &str| -> Option<String> { metadata.custom(key) };
    let get_i32 = |key: &str, default: i32| -> i32 {
        get_str(key)
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(default)
    };
    let parse_float_vec = |key: &str| -> Result<Vec<f32>> {
        let raw = get_str(key).with_context(|| format!("missing metadata key '{key}'"))?;
        raw.split(',')
            .map(|s| {
                s.trim()
                    .parse::<f32>()
                    .with_context(|| format!("invalid float in '{key}': '{s}'"))
            })
            .collect()
    };
    let neg_mean = parse_float_vec("neg_mean")?;
    let inv_stddev = parse_float_vec("inv_stddev")?;
    Ok(SenseVoiceMetadata {
        vocab_size: get_i32("vocab_size", 25055),
        blank_id: get_i32("blank_id", 0),
        lfr_window_size: get_i32("lfr_window_size", 7),
        lfr_window_shift: get_i32("lfr_window_shift", 6),
        normalize_samples: get_i32("normalize_samples", 0) != 0,
        neg_mean,
        inv_stddev,
        lang_auto: get_i32("lang_auto", 0),
        lang_zh: get_i32("lang_zh", 3),
        lang_en: get_i32("lang_en", 4),
        lang_ja: get_i32("lang_ja", 11),
        lang_ko: get_i32("lang_ko", 12),
        lang_yue: get_i32("lang_yue", 7),
        with_itn: get_i32("with_itn", 14),
        without_itn: get_i32("without_itn", 15),
    })
}

// ── Tokens loader ────────────────────────────────────────────────────────────

/// Load a sherpa-onnx-style `tokens.txt` file into a token_id → symbol map.
///
/// Each line: `symbol_text  integer_id` (whitespace-separated).
pub fn load_tokens(path: &Path) -> Result<HashMap<i64, String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read tokens file: {}", path.display()))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.rsplitn(2, |c: char| c.is_ascii_whitespace());
        let id_str = match parts.next() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        let symbol = match parts.next() {
            Some(s) if !s.is_empty() => s.trim(),
            _ => continue,
        };
        let id: i64 = id_str
            .parse()
            .with_context(|| format!("invalid token id '{id_str}' in tokens.txt"))?;
        map.insert(id, symbol.to_string());
    }
    Ok(map)
}

// ── Mel filterbank construction ──────────────────────────────────────────────

/// Kaldi mel scale (not HTK): mel(f) = 1127 * ln(1 + f/700)
fn hz_to_mel(hz: f32) -> f32 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * ((mel / 1127.0).exp() - 1.0)
}

/// Build 80 triangular mel filterbank weights: [NUM_MEL_BINS][NUM_FFT_BINS].
/// Uses Kaldi mel scale with slaney normalization.
fn build_mel_filterbank() -> Vec<Vec<f32>> {
    let mel_low = hz_to_mel(LOW_FREQ);
    let mel_high = hz_to_mel(HIGH_FREQ);
    let mel_step = (mel_high - mel_low) / (NUM_MEL_BINS as f32 + 1.0);

    // Center frequencies in mel and Hz
    let mel_centers: Vec<f32> = (0..NUM_MEL_BINS + 2)
        .map(|i| mel_low + mel_step * i as f32)
        .collect();
    let hz_centers: Vec<f32> = mel_centers.iter().map(|&m| mel_to_hz(m)).collect();

    let freq_resolution = SAMPLE_RATE / FFT_SIZE as f32; // 31.25 Hz

    let mut filters = vec![vec![0.0f32; NUM_FFT_BINS]; NUM_MEL_BINS];
    for m in 0..NUM_MEL_BINS {
        let f_left = hz_centers[m];
        let f_center = hz_centers[m + 1];
        let f_right = hz_centers[m + 2];

        // Slaney normalization: 2 / (f_right - f_left)
        let slaney_norm = 2.0 / (f_right - f_left);

        for (k, filter_bin) in filters[m].iter_mut().enumerate() {
            let freq = k as f32 * freq_resolution;
            let weight = if freq >= f_left && freq <= f_center {
                (freq - f_left) / (f_center - f_left)
            } else if freq > f_center && freq <= f_right {
                (f_right - freq) / (f_right - f_center)
            } else {
                0.0
            };
            *filter_bin = weight * slaney_norm;
        }
    }
    filters
}

/// Build a Hamming window of length N: w[n] = 0.54 - 0.46 * cos(2πn / (N-1))
fn hamming_window(n: usize) -> Vec<f32> {
    let nm1 = (n - 1) as f32;
    (0..n)
        .map(|i| 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / nm1).cos())
        .collect()
}

/// Compute 80-dim log-mel filterbank features from raw f32 samples.
/// Samples must be i16 values cast to f32 (NOT normalized to [-1,1]).
/// Returns Vec of frames, each frame is 80-dim.
pub fn compute_fbank(samples: &[f32]) -> Vec<Vec<f32>> {
    if samples.len() < FRAME_LENGTH_SAMPLES {
        return Vec::new();
    }
    let num_frames = (samples.len() - FRAME_LENGTH_SAMPLES) / FRAME_SHIFT_SAMPLES + 1;
    let mel_filters = build_mel_filterbank();
    let window = hamming_window(FRAME_LENGTH_SAMPLES);

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    let mut features = Vec::with_capacity(num_frames);

    for i in 0..num_frames {
        let start = i * FRAME_SHIFT_SAMPLES;
        let raw_frame = &samples[start..start + FRAME_LENGTH_SAMPLES];

        // 1. DC offset removal: subtract mean from frame
        let mean: f32 = raw_frame.iter().sum::<f32>() / FRAME_LENGTH_SAMPLES as f32;
        let mut frame: Vec<f32> = raw_frame.iter().map(|&s| s - mean).collect();

        // 2. Pre-emphasis: y[n] = x[n] - 0.97 * x[n-1]
        for n in (1..frame.len()).rev() {
            frame[n] -= PRE_EMPHASIS * frame[n - 1];
        }
        frame[0] *= 1.0 - PRE_EMPHASIS;

        // 3. Apply Hamming window
        for (s, w) in frame.iter_mut().zip(window.iter()) {
            *s *= w;
        }

        // 4. Zero-pad to FFT_SIZE and compute FFT
        let mut fft_buf: Vec<Complex<f32>> = frame.iter().map(|&x| Complex::new(x, 0.0)).collect();
        fft_buf.resize(FFT_SIZE, Complex::new(0.0, 0.0));
        fft.process(&mut fft_buf);

        // 5. Power spectrum: |X[k]|^2 for k=0..256 (257 bins)
        let power_spec: Vec<f32> = fft_buf[..NUM_FFT_BINS]
            .iter()
            .map(|c| c.norm_sqr())
            .collect();

        // 6. Mel filterbank + log
        let mut mel_energies = Vec::with_capacity(NUM_MEL_BINS);
        for filter in &mel_filters {
            let energy: f32 = filter
                .iter()
                .zip(power_spec.iter())
                .map(|(f, p)| f * p)
                .sum();
            mel_energies.push(energy.max(ENERGY_FLOOR).ln());
        }

        features.push(mel_energies);
    }

    features
}

// ── LFR (Low Frame Rate) concatenation ────────────────────────────────────────

/// Apply LFR concatenation (sherpa-onnx style, no padding).
/// Concatenates `window_size` consecutive 80-dim frames with `window_shift` stride.
/// Output dimension: window_size * 80 = 560.
pub fn apply_lfr(fbank: &[Vec<f32>], window_size: usize, window_shift: usize) -> Vec<Vec<f32>> {
    let in_frames = fbank.len();
    if in_frames < window_size {
        return Vec::new();
    }
    let out_frames = (in_frames - window_size) / window_shift + 1;
    let out_dim = window_size * NUM_MEL_BINS;
    let mut result = Vec::with_capacity(out_frames);
    for i in 0..out_frames {
        let start = i * window_shift;
        let mut frame = Vec::with_capacity(out_dim);
        for j in 0..window_size {
            frame.extend_from_slice(&fbank[start + j]);
        }
        result.push(frame);
    }
    result
}
// ── CMVN normalization ────────────────────────────────────────────────────────
/// Apply CMVN: normalized = (feature + neg_mean) * inv_stddev, element-wise.
pub fn apply_cmvn(lfr_frames: &mut [Vec<f32>], neg_mean: &[f32], inv_stddev: &[f32]) {
    for frame in lfr_frames.iter_mut() {
        for (i, val) in frame.iter_mut().enumerate() {
            *val = (*val + neg_mean[i]) * inv_stddev[i];
        }
    }
}
// ── CTC greedy decoding ──────────────────────────────────────────────────────
/// CTC greedy decode: argmax per frame, skip blanks and consecutive duplicates.
/// Returns deduplicated token IDs (excluding blank_id).
pub fn ctc_greedy_decode(
    logits: &[f32],
    num_frames: usize,
    vocab_size: usize,
    blank_id: i64,
) -> Vec<i64> {
    let mut result = Vec::new();
    let mut prev_id: i64 = -1;
    for t in 0..num_frames {
        let frame_start = t * vocab_size;
        let frame_end = frame_start + vocab_size;
        let frame_logits = &logits[frame_start..frame_end];
        let y = frame_logits
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx as i64)
            .unwrap_or(blank_id);
        if y != blank_id && y != prev_id {
            result.push(y);
        }
        prev_id = y;
    }
    result
}
// ── Main transcription function ──────────────────────────────────────────────
/// Resolve a language string to the corresponding model metadata language ID.
fn resolve_language_id(metadata: &SenseVoiceMetadata, language: &str) -> i32 {
    match language {
        "zh" => metadata.lang_zh,
        "en" => metadata.lang_en,
        "ja" => metadata.lang_ja,
        "ko" => metadata.lang_ko,
        "yue" => metadata.lang_yue,
        _ => metadata.lang_auto,
    }
}

/// Run the full SenseVoice speech-to-text pipeline.
///
/// `samples` must be i16 values cast to f32 (NOT normalized to [-1,1]).
/// `sample_rate` must be 16000.
#[allow(clippy::implicit_hasher)]
pub fn transcribe_sensevoice(
    session: &mut Session,
    metadata: &SenseVoiceMetadata,
    tokens: &HashMap<i64, String>,
    samples: &[f32],
    sample_rate: u32,
    language: &str,
) -> Result<String> {
    if sample_rate != 16000 {
        bail!("SenseVoice requires 16kHz audio, got {sample_rate}Hz");
    }
    if samples.len() < FRAME_LENGTH_SAMPLES {
        bail!("Audio too short for feature extraction (need at least 400 samples)");
    }
    // 1. Compute 80-dim log-mel Fbank features
    let fbank = compute_fbank(samples);
    if fbank.is_empty() {
        bail!("Fbank produced zero frames");
    }

    // 2. Apply LFR concatenation → 560-dim
    let lfr_win = metadata.lfr_window_size.unsigned_abs() as usize;
    let lfr_shift = metadata.lfr_window_shift.unsigned_abs() as usize;
    let mut lfr_frames = apply_lfr(&fbank, lfr_win, lfr_shift);
    if lfr_frames.is_empty() {
        bail!("LFR produced zero frames (input too short)");
    }

    // 3. Apply CMVN normalization
    apply_cmvn(&mut lfr_frames, &metadata.neg_mean, &metadata.inv_stddev);
    // 4. Flatten LFR frames into a contiguous f32 buffer for ONNX input
    let t_lfr = lfr_frames.len();
    let feat_dim = lfr_frames[0].len();
    let mut flat_features: Vec<f32> = Vec::with_capacity(t_lfr * feat_dim);
    for frame in &lfr_frames {
        flat_features.extend_from_slice(frame);
    }
    // 5. Build ONNX input tensors
    let lang_id = resolve_language_id(metadata, language);
    let features_shape = vec![1i64, t_lfr as i64, feat_dim as i64];
    let features_tensor =
        ort::value::Tensor::from_array((features_shape, flat_features.into_boxed_slice()))
            .context("failed to create features tensor")?;
    let length_tensor = ort::value::Tensor::from_array((
        vec![1i64],
        vec![i32::try_from(t_lfr).unwrap_or(i32::MAX)].into_boxed_slice(),
    ))
    .context("failed to create features_length tensor")?;
    let language_tensor =
        ort::value::Tensor::from_array((vec![1i64], vec![lang_id].into_boxed_slice()))
            .context("failed to create language tensor")?;
    let text_norm_tensor =
        ort::value::Tensor::from_array((vec![1i64], vec![metadata.without_itn].into_boxed_slice()))
            .context("failed to create text_norm tensor")?;
    // 6. Run ONNX inference
    let inputs = ort::inputs![
        "speech" => features_tensor,
        "speech_lengths" => length_tensor,
        "language" => language_tensor,
        "text_norm" => text_norm_tensor,
    ];
    let outputs = session.run(inputs).context("ONNX inference failed")?;
    // 7. Extract logits from output
    let (logits_shape, logits_data) = outputs[0]
        .try_extract_tensor::<f32>()
        .context("failed to extract logits tensor")?;
    let out_frames = logits_shape.get(1).copied().unwrap_or(0).unsigned_abs() as usize;
    let vocab = logits_shape.get(2).copied().unwrap_or(0).unsigned_abs() as usize;
    // 8. CTC greedy decode
    let token_ids = ctc_greedy_decode(logits_data, out_frames, vocab, i64::from(metadata.blank_id));
    // 9. Skip first 4 prompt tokens (lang, emotion, event, punct) and map to text
    let text_tokens: Vec<&str> = token_ids
        .iter()
        .skip(4)
        .filter_map(|id| tokens.get(id).map(|s| s.as_str()))
        .collect();
    Ok(text_tokens.join(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fbank_dimensions() {
        // 1 second of 16kHz silence → ~98 frames × 80 dims
        // num_frames = (16000 - 400) / 160 + 1 = 15600/160 + 1 = 97 + 1 = 98
        let samples = vec![0.0f32; 16000];
        let fbank = compute_fbank(&samples);
        assert_eq!(fbank.len(), 98, "expected 98 frames for 1s of 16kHz audio");
        assert_eq!(fbank[0].len(), 80, "expected 80 mel bins per frame");
    }

    #[test]
    fn test_fbank_short_audio() {
        // Audio shorter than one frame should return empty
        let samples = vec![0.0f32; 399];
        let fbank = compute_fbank(&samples);
        assert!(fbank.is_empty());
    }
    #[test]
    fn test_lfr_dimensions() {
        // 98 fbank frames with window=7, shift=6:
        // out_frames = (98 - 7) / 6 + 1 = 91/6 + 1 = 15 + 1 = 16
        let fbank: Vec<Vec<f32>> = (0..98).map(|_| vec![0.0f32; 80]).collect();
        let lfr = apply_lfr(&fbank, 7, 6);
        assert_eq!(lfr.len(), 16, "expected 16 LFR frames");
        assert_eq!(lfr[0].len(), 560, "expected 560-dim LFR frames (7 * 80)");
    }
    #[test]
    fn test_lfr_too_short() {
        // Fewer frames than window_size should return empty
        let fbank: Vec<Vec<f32>> = (0..6).map(|_| vec![0.0f32; 80]).collect();
        let lfr = apply_lfr(&fbank, 7, 6);
        assert!(lfr.is_empty());
    }
    #[test]
    fn test_ctc_decode() {
        // Input: [0, 1, 1, 0, 2, 2, 2, 0, 3] with blank_id=0
        // Expected: skip blanks + dedup → [1, 2, 3]
        let vocab_size = 4;
        let num_frames = 9;
        // Build logits where argmax gives [0, 1, 1, 0, 2, 2, 2, 0, 3]
        let mut logits = vec![0.0f32; num_frames * vocab_size];
        let expected_argmax = [0, 1, 1, 0, 2, 2, 2, 0, 3];
        for (t, &target) in expected_argmax.iter().enumerate() {
            logits[t * vocab_size + target] = 1.0;
        }
        let result = ctc_greedy_decode(&logits, num_frames, vocab_size, 0);
        assert_eq!(result, vec![1, 2, 3]);
    }
    #[test]
    fn test_ctc_decode_all_blanks() {
        let vocab_size = 4;
        let num_frames = 5;
        let mut logits = vec![0.0f32; num_frames * vocab_size];
        for t in 0..num_frames {
            logits[t * vocab_size] = 1.0; // argmax = 0 = blank
        }
        let result = ctc_greedy_decode(&logits, num_frames, vocab_size, 0);
        assert!(result.is_empty());
    }
    #[test]
    fn test_load_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.txt");
        std::fs::write(&path, "<blank> 0\n<sos> 1\n<eos> 2\n▁the 42\nhello 1337\n").unwrap();
        let tokens = load_tokens(&path).unwrap();
        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[&0], "<blank>");
        assert_eq!(tokens[&1], "<sos>");
        assert_eq!(tokens[&42], "▁the");
        assert_eq!(tokens[&1337], "hello");
    }
    #[test]
    fn test_load_tokens_with_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.txt");
        std::fs::write(&path, "# comment line\n\n<blank> 0\nhello 1\n  \n").unwrap();
        let tokens = load_tokens(&path).unwrap();
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[&0], "<blank>");
        assert_eq!(tokens[&1], "hello");
    }
    #[test]
    fn test_mel_filterbank_shape() {
        let filters = build_mel_filterbank();
        assert_eq!(filters.len(), 80, "expected 80 mel filters");
        assert_eq!(filters[0].len(), 257, "expected 257 FFT bins per filter");
    }
    #[test]
    fn test_cmvn_normalization() {
        let mut frames = vec![vec![1.0f32; 4]; 2];
        let neg_mean = vec![-1.0f32; 4]; // neg_mean = -mean, so feature + neg_mean = feature - mean
        let inv_stddev = vec![2.0f32; 4];
        apply_cmvn(&mut frames, &neg_mean, &inv_stddev);
        // (1.0 + (-1.0)) * 2.0 = 0.0
        for frame in &frames {
            for &val in frame {
                assert!((val - 0.0).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn test_fbank_nonsilence_produces_varied_output() {
        // Sine wave at 440 Hz should produce non-uniform mel energies
        let num_samples = 16000;
        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f32 / 16000.0;
                (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 10000.0
            })
            .collect();
        let fbank = compute_fbank(&samples);
        assert_eq!(fbank.len(), 98);
        // Not all values should be the same (energy floor)
        let first_frame = &fbank[0];
        let has_variation = first_frame.windows(2).any(|w| (w[0] - w[1]).abs() > 0.01);
        assert!(
            has_variation,
            "fbank should have varied mel energies for a sine wave"
        );
    }
}
