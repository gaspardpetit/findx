use std::fs;

use anyhow::Result;
use camino::Utf8PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct EmbeddingConfig {
    pub provider: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MirrorConfig {
    pub root: Utf8PathBuf,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            root: Utf8PathBuf::from(".findx/raw"),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BusBounds {
    pub source_fs: usize,
    pub mirror_text: usize,
}

impl Default for BusBounds {
    fn default() -> Self {
        Self {
            source_fs: 1024,
            mirror_text: 1024,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BusConfig {
    pub bounds: BusBounds,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            bounds: BusBounds::default(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ExtractConfig {
    pub pool_size: usize,
    #[serde(default = "default_jobs_bound")]
    pub jobs_bound: usize,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            pool_size: 4,
            jobs_bound: default_jobs_bound(),
        }
    }
}

fn default_jobs_bound() -> usize {
    2048
}

#[derive(Debug, Deserialize, Clone)]
pub struct RetentionConfig {
    #[serde(default = "default_events_days")]
    pub events_days: u64,
    #[serde(default = "default_jobs_keep_per_file")]
    pub jobs_keep_per_file: usize,
    #[serde(default = "default_jobs_failed_days")]
    pub jobs_failed_days: u64,
    #[serde(default = "default_files_tombstone_days")]
    pub files_tombstone_days: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            events_days: default_events_days(),
            jobs_keep_per_file: default_jobs_keep_per_file(),
            jobs_failed_days: default_jobs_failed_days(),
            files_tombstone_days: default_files_tombstone_days(),
        }
    }
}

fn default_events_days() -> u64 {
    14
}

fn default_jobs_keep_per_file() -> usize {
    3
}

fn default_jobs_failed_days() -> u64 {
    14
}

fn default_files_tombstone_days() -> u64 {
    30
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
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub allow_offline_hydration: bool,
    pub commit_interval_secs: u64,
    pub guard_interval_secs: u64,
    pub default_language: String,
    #[serde(default = "default_extractor_cmd")]
    pub extractor_cmd: String,
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub mirror: MirrorConfig,
    #[serde(default)]
    pub bus: BusConfig,
    #[serde(default)]
    pub extract: ExtractConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db: Utf8PathBuf::from(".findx/catalog.db"),
            tantivy_index: Utf8PathBuf::from(".findx/idx"),
            roots: vec![Utf8PathBuf::from(".")],
            include: vec![
                "**/*.pdf".into(),
                "**/*.docx".into(),
                "**/*.md".into(),
                "**/*.txt".into(),
            ],
            exclude: vec!["**/.git/**".into(), "**/~$*".into()],
            max_file_size_mb: 200,
            follow_symlinks: false,
            include_hidden: false,
            allow_offline_hydration: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            extractor_cmd: default_extractor_cmd(),
            embedding: EmbeddingConfig {
                provider: "disabled".into(),
            },
            mirror: MirrorConfig::default(),
            bus: BusConfig::default(),
            extract: ExtractConfig::default(),
            retention: RetentionConfig::default(),
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

fn default_extractor_cmd() -> String {
    "docling --to text".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mirror_root() {
        let cfg = Config::default();
        assert_eq!(cfg.mirror.root, Utf8PathBuf::from(".findx/raw"));
    }

    #[test]
    fn default_retention() {
        let cfg = Config::default();
        assert_eq!(cfg.retention.events_days, 14);
        assert_eq!(cfg.retention.jobs_keep_per_file, 3);
        assert_eq!(cfg.retention.jobs_failed_days, 14);
        assert_eq!(cfg.retention.files_tombstone_days, 30);
    }
}
