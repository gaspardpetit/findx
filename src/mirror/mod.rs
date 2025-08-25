use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use crossbeam_channel::RecvTimeoutError;
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::bus::EventBus;
use crate::config::Config;
use crate::db;
use crate::events::{MirrorEvent, PageBlock, SourceEvent};

const TOKENS_PER_CHUNK: usize = 200;

#[derive(Serialize)]
struct Meta<'a> {
    v: u8,
    file_uid: &'a str,
    path: &'a Utf8Path,
    content_hash: &'a str,
    extractor: &'a str,
    extractor_version: &'a str,
    page_count: usize,
    lang: &'a str,
    created_ts: String,
}

#[derive(Serialize)]
struct PageSpan {
    page: u32,
    start_char: usize,
    end_char: usize,
}

#[derive(Serialize)]
struct ByteSpan {
    start: usize,
    end: usize,
}

#[derive(Serialize)]
struct Chunk<'a> {
    v: u8,
    chunk_id: String,
    file_uid: &'a str,
    content_hash: &'a str,
    order: u64,
    text: &'a str,
    page_spans: Vec<PageSpan>,
    byte_span: ByteSpan,
    tokens_est: usize,
}

/// Run the mirror builder, consuming `ExtractionCompleted` events and writing
/// mirror artifacts under `mirror.root`.
pub fn run(bus: EventBus, cfg: &Config, stop: &AtomicBool) -> Result<()> {
    let rx = bus.subscribe_source();
    let conn = Arc::new(Mutex::new(db::open(&cfg.db)?));
    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(env) => match env.data {
                SourceEvent::ExtractionCompleted {
                    file_uid,
                    content_hash,
                    extractor,
                    extractor_version,
                    pages,
                } => {
                    handle_extraction(
                        &bus,
                        &conn,
                        cfg,
                        &file_uid,
                        &content_hash,
                        &extractor,
                        &extractor_version,
                        &pages,
                    )?;
                }
                _ => {}
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn handle_extraction(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    cfg: &Config,
    file_uid: &str,
    content_hash: &str,
    extractor: &str,
    extractor_version: &str,
    pages: &[PageBlock],
) -> Result<()> {
    let path_str: String = {
        let c = conn.lock().unwrap();
        c.query_row(
            "SELECT realpath FROM files WHERE inode_hint=?1",
            params![file_uid],
            |r| r.get(0),
        )?
    };
    let path = Utf8PathBuf::from(path_str);
    let rel = relativize(&path, &cfg.roots);
    let dir = cfg.mirror.root.join(&rel);
    fs::create_dir_all(&dir)?;

    write_meta(
        &dir,
        &rel,
        file_uid,
        content_hash,
        extractor,
        extractor_version,
        pages.len(),
        &cfg.default_language,
    )?;

    {
        let conn = conn.lock().unwrap();
        let ts = now();
        conn.execute(
            "INSERT OR REPLACE INTO mirror_docs (file_uid, content_hash, path, updated_ts) VALUES (?1, ?2, ?3, ?4)",
            params![file_uid, content_hash, rel.as_str(), ts],
        )?;
        conn.execute(
            "DELETE FROM mirror_chunks WHERE file_uid=?1",
            params![file_uid],
        )?;
    }

    write_chunks(bus, conn, &dir, file_uid, content_hash, pages)?;
    bus.publish_mirror(MirrorEvent::MirrorDocUpserted {
        file_uid: file_uid.to_string(),
        content_hash: content_hash.to_string(),
    })?;
    Ok(())
}

fn write_meta(
    dir: &Utf8PathBuf,
    rel: &Utf8PathBuf,
    file_uid: &str,
    content_hash: &str,
    extractor: &str,
    extractor_version: &str,
    page_count: usize,
    lang: &str,
) -> Result<()> {
    let meta_path = dir.join("meta.json");
    let tmp = dir.join("meta.json.tmp");
    let meta = Meta {
        v: 1,
        file_uid,
        path: rel.as_path(),
        content_hash,
        extractor,
        extractor_version,
        page_count,
        lang,
        created_ts: Utc::now().to_rfc3339(),
    };
    let mut f = File::create(&tmp)?;
    serde_json::to_writer(&mut f, &meta)?;
    f.flush()?;
    fs::rename(&tmp, &meta_path)?;
    Ok(())
}

fn write_chunks(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    dir: &Utf8PathBuf,
    file_uid: &str,
    content_hash: &str,
    pages: &[PageBlock],
) -> Result<()> {
    let chunks_path = dir.join("chunks.jsonl");
    let file = File::create(&chunks_path)?;
    let mut writer = BufWriter::new(file);
    let mut order = 0u64;
    for page in pages {
        let mut idx = 0usize;
        let chars: Vec<char> = page.text.chars().collect();
        while idx < chars.len() {
            let mut end = idx;
            let mut tokens = 0usize;
            while end < chars.len() && tokens < TOKENS_PER_CHUNK {
                if chars[end].is_whitespace() {
                    while end < chars.len() && chars[end].is_whitespace() {
                        end += 1;
                    }
                    tokens += 1;
                } else {
                    end += 1;
                }
            }
            if end == idx {
                break;
            }
            let text: String = chars[idx..end].iter().collect();
            let chunk_id = make_chunk_id(file_uid, content_hash, page.page_no, idx, end, &text);
            let chunk = Chunk {
                v: 1,
                chunk_id: chunk_id.clone(),
                file_uid,
                content_hash,
                order,
                text: &text,
                page_spans: vec![PageSpan {
                    page: page.page_no,
                    start_char: idx,
                    end_char: end,
                }],
                byte_span: ByteSpan {
                    start: page.start + idx,
                    end: page.start + end,
                },
                tokens_est: text.split_whitespace().count(),
            };
            serde_json::to_writer(&mut writer, &chunk)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            {
                let conn = conn.lock().unwrap();
                conn.execute(
                    "INSERT OR REPLACE INTO mirror_chunks (chunk_id, file_uid, ord) VALUES (?1, ?2, ?3)",
                    params![chunk_id, file_uid, order as i64],
                )?;
            }
            bus.publish_mirror(MirrorEvent::MirrorChunkUpserted {
                chunk_id: chunk.chunk_id.clone(),
                file_uid: file_uid.to_string(),
                order,
            })?;
            order += 1;
            idx = end;
        }
    }
    Ok(())
}

fn make_chunk_id(
    file_uid: &str,
    content_hash: &str,
    page_no: u32,
    start: usize,
    end: usize,
    text: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_uid.as_bytes());
    hasher.update(content_hash.as_bytes());
    hasher.update(page_no.to_be_bytes());
    hasher.update(start.to_be_bytes());
    hasher.update(end.to_be_bytes());
    hasher.update(text.as_bytes());
    format!("ch:{:x}", hasher.finalize())
}

fn relativize(path: &Utf8Path, roots: &[Utf8PathBuf]) -> Utf8PathBuf {
    for root in roots {
        if path.starts_with(root) {
            if let Ok(p) = path.strip_prefix(root) {
                return p.to_path_buf();
            }
        }
    }
    path.to_path_buf()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig};
    use std::sync::atomic::AtomicBool;
    use tempfile::tempdir;

    #[test]
    fn writes_meta_and_chunks() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let cfg = crate::config::Config {
            db: root.join("catalog.db"),
            tantivy_index: Utf8PathBuf::from("idx"),
            roots: vec![root.clone()],
            include: vec![],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            extractor_cmd: String::new(),
            embedding: crate::config::EmbeddingConfig {
                provider: "disabled".into(),
            },
            mirror: MirrorConfig {
                root: root.join("raw"),
            },
            bus: BusConfig {
                bounds: BusBounds {
                    source_fs: 8,
                    mirror_text: 8,
                },
            },
            extract: ExtractConfig { pool_size: 1 },
        };
        let conn = db::open(&cfg.db)?;
        conn.execute(
            "INSERT INTO files (realpath, size, mtime_ns, inode_hint, hash, status, created_ts, updated_ts) VALUES (?1,0,0,?2,?3,'active',0,0)",
            params![root.join("a.txt").as_str(), "f1", "h1"],
        )?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
        let rx = bus.subscribe_mirror();
        let stop = Arc::new(AtomicBool::new(false));
        let bus_run = bus.clone();
        let cfg_run = cfg.clone();
        let stop_run = stop.clone();
        std::thread::spawn(move || {
            run(bus_run, &cfg_run, &stop_run).unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        bus.publish_source(SourceEvent::ExtractionCompleted {
            file_uid: "f1".into(),
            content_hash: "h1".into(),
            extractor: "builtin".into(),
            extractor_version: "".into(),
            pages: vec![PageBlock {
                page_no: 1,
                text: "hello world".into(),
                start: 0,
                end: 2,
            }],
        })?;
        // expect chunk and doc events
        rx.recv_timeout(std::time::Duration::from_millis(500))?;
        rx.recv_timeout(std::time::Duration::from_millis(500))?;
        let meta_path = cfg.mirror.root.join("a.txt").join("meta.json");
        let chunks_path = cfg.mirror.root.join("a.txt").join("chunks.jsonl");
        assert!(meta_path.exists());
        assert!(chunks_path.exists());
        Ok(())
    }
}
