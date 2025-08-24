use std::fs::OpenOptions;
use std::io::Write;

use anyhow::Result;
use camino::Utf8PathBuf;

use crate::config::Config;

/// Append a line to the index log under `.findx/index.log`.
pub fn append(cfg: &Config, msg: &str) -> Result<()> {
    let base: Utf8PathBuf = cfg
        .db
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| Utf8PathBuf::from("."));
    std::fs::create_dir_all(base.as_std_path())?;
    let log_path = base.join("index.log");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path.as_std_path())?;
    writeln!(file, "{}", msg)?;
    Ok(())
}
