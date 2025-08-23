//! Document content extraction via Python sidecar.

use anyhow::Result;
use camino::Utf8Path;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::config::Config;

#[derive(Serialize)]
struct ExtractRequest<'a> {
    path: &'a str,
    lang_hint: &'a str,
}

#[derive(Deserialize)]
struct ExtractResponse {
    ok: bool,
    extractor: String,
    version: String,
    lang: Option<String>,
    pages: Option<i64>,
    markdown: Option<String>,
    text: Option<String>,
    ocr: Option<bool>,
}

/// Call the extraction sidecar for `path` and store results in the DB.
pub fn extract_file(conn: &Connection, file_id: i64, path: &Utf8Path, cfg: &Config) -> Result<()> {
    let req = ExtractRequest {
        path: path.as_str(),
        lang_hint: &cfg.default_language,
    };
    let client = reqwest::blocking::Client::new();
    let resp = client.post(&cfg.extractor_url).json(&req).send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "extraction request failed");
            return Ok(());
        }
    };
    let parsed: ExtractResponse = match resp.json() {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "invalid extraction response");
            return Ok(());
        }
    };
    if parsed.ok {
        let now_ts = now();
        conn.execute(
            "INSERT OR REPLACE INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                file_id,
                parsed.extractor,
                parsed.version,
                parsed.lang,
                parsed.pages,
                parsed.markdown,
                parsed.text,
                parsed.ocr.unwrap_or(false) as i32,
                now_ts,
            ],
        )?;
    }
    Ok(())
}

fn now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
