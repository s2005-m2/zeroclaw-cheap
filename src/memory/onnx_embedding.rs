#![cfg(feature = "local-embedding")]

use std::path::Path;

use anyhow::Context;
use async_trait::async_trait;
use ort::session::Session;
use tokenizers::Tokenizer;

use super::embeddings::EmbeddingProvider;

pub struct OnnxEmbeddingProvider {
    session: std::sync::Arc<std::sync::Mutex<Session>>,
    tokenizer: std::sync::Arc<Tokenizer>,
    pub dims: usize,
}

impl OnnxEmbeddingProvider {
    pub fn new(model_dir: &Path) -> anyhow::Result<Self> {
        Self::new_with_dims(model_dir, 768)
    }

    pub fn new_with_dims(model_dir: &Path, dims: usize) -> anyhow::Result<Self> {
        let session = Session::builder()
            .context("failed to create ORT session builder")?
            .with_intra_threads(2)
            .context("failed to set intra-op threads")?
            .commit_from_file(model_dir.join("model_quantized.onnx"))
            .context("failed to load ONNX model")?;

        let tokenizer = Tokenizer::from_file(model_dir.join("tokenizer.json"))
            .map_err(|e| anyhow::anyhow!("failed to load tokenizer: {e}"))?;

        Ok(Self {
            session: std::sync::Arc::new(std::sync::Mutex::new(session)),
            tokenizer: std::sync::Arc::new(tokenizer),
            dims,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for OnnxEmbeddingProvider {
    fn name(&self) -> &str {
        "onnx"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| format!("title: none | text: {t}"))
            .collect();

        let session = self.session.clone();
        let tokenizer = self.tokenizer.clone();

        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<f32>>> {
            let encodings = tokenizer
                .encode_batch(prefixed, true)
                .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

            let batch = encodings.len();
            let seq_len = encodings
                .iter()
                .map(|e| e.get_ids().len())
                .max()
                .unwrap_or(0);

            let mut input_ids = vec![0i64; batch * seq_len];
            let mut attention_mask = vec![0i64; batch * seq_len];

            for (i, enc) in encodings.iter().enumerate() {
                for (j, (&id, &m)) in enc
                    .get_ids()
                    .iter()
                    .zip(enc.get_attention_mask().iter())
                    .enumerate()
                {
                    input_ids[i * seq_len + j] = id as i64;
                    attention_mask[i * seq_len + j] = m as i64;
                }
            }

            let shape = vec![batch as i64, seq_len as i64];
            let ids_tensor =
                ort::value::Tensor::from_array((shape.clone(), input_ids.into_boxed_slice()))
                    .context("input_ids tensor")?;
            let mask_tensor =
                ort::value::Tensor::from_array((shape, attention_mask.into_boxed_slice()))
                    .context("attention_mask tensor")?;

            let inputs = ort::inputs![
                "input_ids" => ids_tensor,
                "attention_mask" => mask_tensor,
            ];

            let mut sess = session
                .lock()
                .map_err(|e| anyhow::anyhow!("session lock poisoned: {e}"))?;
            let outputs = sess.run(inputs).context("ORT inference failed")?;

            let (emb_shape, emb_data) = outputs["sentence_embedding"]
                .try_extract_tensor::<f32>()
                .context("failed to extract sentence_embedding")?;

            let cols = emb_shape.get(1).copied().unwrap_or(0) as usize;
            Ok(emb_data
                .chunks(cols)
                .map(|row| row.to_vec())
                .collect::<Vec<_>>())
        })
        .await
        .context("spawn_blocking panicked")?
    }
}
