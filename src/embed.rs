use anyhow::{anyhow, bail, Context, Result};
use camino::Utf8PathBuf;
use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use once_cell::sync::OnceCell;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::{env, fs, sync::Mutex};

/// Local embedder backed by fastembed.
pub struct LocalEmbedder {
    model: Mutex<TextEmbedding>,
}

impl LocalEmbedder {
    pub fn new() -> Result<Self> {
        let model_name = env::var("EMBEDDING_MODEL")
            .unwrap_or_else(|_| EmbeddingModel::MxbaiEmbedLargeV1.to_string());

        let model = if let Some(m) = load_local_model(&model_name)? {
            m
        } else {
            let parsed = model_name
                .parse::<EmbeddingModel>()
                .map_err(|_| anyhow!("unsupported embedding model '{}'", model_name))?;
            TextEmbedding::try_new(InitOptions::new(parsed).with_show_download_progress(true))
                .with_context(|| {
                    format!(
                        "failed to initialize fastembed TextEmbedding for '{}'",
                        model_name
                    )
                })?
        };
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

fn load_local_model(name: &str) -> Result<Option<TextEmbedding>> {
    let base = Utf8PathBuf::from("models").join(name);
    if !base.exists() {
        return Ok(None);
    }

    let onnx = {
        let standard = base.join("model.onnx");
        if standard.exists() {
            standard
        } else {
            let quant = base.join("model_uint8.onnx");
            if quant.exists() {
                quant
            } else {
                return Ok(None);
            }
        }
    };

    let tokenizer = TokenizerFiles {
        tokenizer_file: fs::read(base.join("tokenizer.json"))
            .context("failed to read tokenizer.json")?,
        config_file: fs::read(base.join("config.json")).context("failed to read config.json")?,
        special_tokens_map_file: fs::read(base.join("special_tokens_map.json"))
            .context("failed to read special_tokens_map.json")?,
        tokenizer_config_file: fs::read(base.join("tokenizer_config.json"))
            .context("failed to read tokenizer_config.json")?,
    };

    let ud_model = UserDefinedEmbeddingModel::new(
        fs::read(&onnx).context("failed to read model file")?,
        tokenizer,
    );
    let model = TextEmbedding::try_new_from_user_defined(ud_model, InitOptionsUserDefined::new())
        .context("failed to initialize local TextEmbedding")?;
    Ok(Some(model))
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

    pub fn embed(&self, texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
        let inputs: Vec<String> = texts.iter().map(|t| t.as_ref().to_string()).collect();
        let req = ExternalRequest {
            input: &inputs,
            model: self.model_hint.clone(),
        };
        let client = Client::new();
        let mut rb = client.post(&self.url).json(&req);
        if let Some(k) = &self.api_key {
            rb = rb.header("Authorization", format!("Bearer {}", k));
        }
        let resp = rb.send().context("failed to call external embedder")?;
        if resp.status() != StatusCode::OK {
            bail!("external embedder returned {}", resp.status());
        }
        let parsed: ExternalResponse =
            resp.json().context("invalid JSON from external embedder")?;
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

    pub fn embed(&self, texts: &[impl AsRef<str>]) -> Result<Vec<Vec<f32>>> {
        match self {
            Embedder::Local(m) => m.embed(texts),
            Embedder::External(m) => m.embed(texts),
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
    embedder.embed(texts)
}
