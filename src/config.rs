use std::fs;

use anyhow::Result;
use camino::Utf8PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct EmbeddingConfig {
    pub provider: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub db: Utf8PathBuf,
    pub tantivy_index: Utf8PathBuf,
    pub roots: Vec<Utf8PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_file_size_mb: u64,
    pub follow_symlinks: bool,
    pub commit_interval_secs: u64,
    pub guard_interval_secs: u64,
    pub default_language: String,
    pub embedding: EmbeddingConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db: Utf8PathBuf::from("state/catalog.db"),
            tantivy_index: Utf8PathBuf::from("state/idx"),
            roots: vec![Utf8PathBuf::from("/data/a")],
            include: vec![
                "**/*.pdf".into(),
                "**/*.docx".into(),
                "**/*.md".into(),
                "**/*.txt".into(),
            ],
            exclude: vec!["**/.git/**".into(), "**/~$*".into()],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            embedding: EmbeddingConfig { provider: "disabled".into() },
        }
    }
}

impl Config {
    pub fn load(path: &Utf8PathBuf) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&content)?;
        Ok(cfg)
    }
}
