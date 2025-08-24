//! Document content extraction via external command or builtin reader.

use std::{fs, process::Command};

use anyhow::{bail, Context, Result};
use camino::Utf8Path;
use rusqlite::Connection;
use tracing::warn;

use crate::config::Config;

const PLAINTEXT_EXTS: &[&str] = &["txt", "md", "rs", "toml", "json", "cpp", "c", "h", "hpp"];

/// Extract text from `path` and store results in the DB.
///
/// Plain text files are read directly. Other formats are passed to the
/// configured extractor command which should write extracted text to stdout.
pub fn extract_file(conn: &Connection, file_id: i64, path: &Utf8Path, cfg: &Config) -> Result<()> {
    let text = if is_plaintext(path) {
        match fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(%path, error = %e, "failed to read text file");
                None
            }
        }
    } else if cfg.extractor_cmd.trim().is_empty() {
        warn!(%path, "no extractor command configured; skipping");
        None
    } else {
        match run_command(&cfg.extractor_cmd, path) {
            Ok(t) => Some(t),
            Err(e) => {
                warn!(%path, error = %e, "extractor command failed");
                None
            }
        }
    };

    if let Some(text) = text {
        let now_ts = now();
        let extractor = if is_plaintext(path) {
            "builtin".to_string()
        } else {
            shell_words::split(&cfg.extractor_cmd)
                .ok()
                .and_then(|parts| parts.into_iter().next())
                .unwrap_or_else(|| "cmd".to_string())
        };
        conn.execute(
            "INSERT OR REPLACE INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                file_id,
                extractor,
                "",
                Option::<String>::None,
                Option::<i64>::None,
                Option::<String>::None,
                text,
                0,
                now_ts,
            ],
        )?;
    }

    Ok(())
}

fn is_plaintext(path: &Utf8Path) -> bool {
    path.extension()
        .map(|e| PLAINTEXT_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn run_command(cmd: &str, path: &Utf8Path) -> Result<String> {
    let parts = shell_words::split(cmd).context("parse extractor_cmd")?;
    let prog = parts.first().context("empty extractor_cmd")?;
    let output = Command::new(prog)
        .args(&parts[1..])
        .arg(path.as_str())
        .output()?;
    if !output.status.success() {
        bail!("command exited with status {:?}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
