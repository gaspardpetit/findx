use anyhow::{bail, Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::OnceCell;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{env, sync::Mutex};

/// Local embedder backed by fastembed.
pub struct LocalEmbedder {
    model: Mutex<TextEmbedding>,
}

impl LocalEmbedder {
    pub fn new() -> Result<Self> {
        let model_name =
            env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "MxbaiEmbedLargeV1".to_string());
        let parsed = model_name
            .parse::<EmbeddingModel>()
            .unwrap_or(EmbeddingModel::MxbaiEmbedLargeV1);
        let model =
            TextEmbedding::try_new(InitOptions::new(parsed).with_show_download_progress(true))
                .context("failed to initialize fastembed TextEmbedding")?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }

    pub fn embed(&self, texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
        let docs: Vec<String> = texts.iter().map(|t| t.as_ref().to_string()).collect();
        let mut model = self.model.lock().unwrap();
        let embs = model
            .embed(docs, None)
            .context("fastembed inference failed")?;
        Ok(embs)
    }

    pub fn print_supported() {
        eprintln!(
            "fastembed supported models: {:?}",
            TextEmbedding::list_supported_models()
        );
    }
}

#[derive(Serialize)]
struct ExternalRequest<'a> {
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Deserialize)]
struct ExternalResponse {
    data: Vec<ExternalEmbeddingItem>,
}

#[derive(Deserialize)]
struct ExternalEmbeddingItem {
    embedding: Vec<f32>,
}

pub struct ExternalEmbedder {
    url: String,
    api_key: Option<String>,
    model_hint: Option<String>,
}

impl ExternalEmbedder {
    pub fn new() -> Result<Self> {
        let url = env::var("EMBEDDING_URL")
            .context("EMBEDDING_URL is required for external embedding")?;
        let api_key = env::var("EMBEDDING_API_KEY").ok();
        let model_hint = env::var("EMBEDDING_MODEL").ok();
        Ok(Self {
            url,
            api_key,
            model_hint,
        })
    }

    pub async fn embed(&self, texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
        let inputs: Vec<String> = texts.iter().map(|t| t.as_ref().to_string()).collect();
        let req = ExternalRequest {
            input: &inputs,
            model: self.model_hint.clone(),
        };
        let client = reqwest::Client::new();
        let mut rb = client.post(&self.url).json(&req);
        if let Some(k) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", k));
        }
        let resp = rb
            .send()
            .await
            .context("failed to call external embedder")?;
        if resp.status() != StatusCode::OK {
            bail!("external embedder returned {}", resp.status());
        }
        let parsed: ExternalResponse = resp
            .json()
            .await
            .context("invalid JSON from external embedder")?;
        let out = parsed.data.into_iter().map(|i| i.embedding).collect();
        Ok(out)
    }
}

pub enum Embedder {
    Local(LocalEmbedder),
    External(ExternalEmbedder),
}

impl Embedder {
    pub fn from_env() -> Result<Self> {
        if env::var("EMBEDDING_URL").is_ok() {
            Ok(Self::External(ExternalEmbedder::new()?))
        } else {
            Ok(Self::Local(LocalEmbedder::new()?))
        }
    }

    pub async fn embed(&self, texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
        match self {
            Embedder::Local(m) => m.embed(texts),
            Embedder::External(m) => m.embed(texts).await,
        }
    }
}

static EMBEDDER: OnceCell<Embedder> = OnceCell::new();

fn get_embedder() -> Result<&'static Embedder> {
    EMBEDDER.get_or_try_init(Embedder::from_env)
}

/// Embed a single text, returning its vector representation.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let res = embed_batch(&[text])?;
    Ok(res.into_iter().next().unwrap())
}

/// Embed a batch of texts.
pub fn embed_batch(texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
    let embedder = get_embedder()?;
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(embedder.embed(texts)),
        Err(_) => {
            let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
            rt.block_on(embedder.embed(texts))
        }
    }
}
