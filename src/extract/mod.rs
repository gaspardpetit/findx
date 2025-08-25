//! Document content extraction via worker pool and external command.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{fs, process::Command};

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use crossbeam_channel::{bounded, Receiver, RecvTimeoutError};
use rusqlite::{params, Connection};

use crate::bus::EventBus;
use crate::config::Config;
use crate::db;
use crate::events::{PageBlock, SourceEvent};

const PLAINTEXT_EXTS: &[&str] = &["txt", "md", "rs", "toml", "json", "cpp", "c", "h", "hpp"];

/// Run the extraction worker pool. Workers consume `ExtractionRequested` events
/// and emit `ExtractionCompleted` or `ExtractionFailed` events.
pub fn run_pool(bus: EventBus, cfg: &Config, stop: &AtomicBool) -> Result<()> {
    let rx_events = bus.subscribe_source();
    let (job_tx, job_rx) = bounded::<String>(cfg.extract.jobs_bound);

    for _ in 0..cfg.extract.pool_size {
        let rx = job_rx.clone();
        let bus_w = bus.clone();
        let cfg_w = cfg.clone();
        let db_path = cfg.db.clone();
        std::thread::spawn(move || worker_loop(rx, bus_w, cfg_w, db_path));
    }

    while !stop.load(Ordering::SeqCst) {
        match rx_events.recv_timeout(Duration::from_millis(100)) {
            Ok(env) => match env.data {
                SourceEvent::ExtractionRequested { file_uid } => {
                    let _ = job_tx.send(file_uid);
                }
                _ => {}
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn worker_loop(rx: Receiver<String>, bus: EventBus, cfg: Config, db_path: Utf8PathBuf) {
    let conn = db::open(&db_path).expect("open db");
    while let Ok(file_uid) = rx.recv() {
        let started_ts = now();
        let path_hash: Result<(Utf8PathBuf, String), anyhow::Error> = (|| {
            let path_str: String = conn.query_row(
                "SELECT realpath FROM files WHERE inode_hint=?1",
                params![file_uid],
                |r| r.get(0),
            )?;
            let path = Utf8PathBuf::from(path_str);
            let content_hash = hash_file(&path)?;
            Ok((path, content_hash))
        })();
        let (path, content_hash) = match path_hash {
            Ok(v) => v,
            Err(e) => {
                let _ = conn.execute(
                    "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts, finished_ts, error) VALUES (?1, '', 'failed', 1, ?2, ?3, ?4)",
                    params![file_uid, started_ts, started_ts, e.to_string()],
                );
                let _ = bus.publish_source(SourceEvent::ExtractionFailed {
                    file_uid: file_uid.clone(),
                    error: e.to_string(),
                });
                continue;
            }
        };
        let inserted = conn
            .execute(
                "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts) VALUES (?1, ?2, 'running', 1, ?3) ON CONFLICT(file_uid, content_hash) DO NOTHING",
                params![file_uid, content_hash, started_ts],
            )
            .unwrap_or(0);
        if inserted == 0 {
            continue;
        }
        match extract_one(&conn, &cfg, &bus, &file_uid, &content_hash, &path) {
            Ok(()) => {
                let finished_ts = now();
                let _ = conn.execute(
                    "UPDATE extract_jobs SET status='done', finished_ts=?3 WHERE file_uid=?1 AND content_hash=?2",
                    params![file_uid, content_hash, finished_ts],
                );
                let _ = conn.execute(
                    "UPDATE files SET hash=?2, updated_ts=?3 WHERE inode_hint=?1",
                    params![file_uid, content_hash, finished_ts],
                );
            }
            Err(e) => {
                let finished_ts = now();
                let _ = conn.execute(
                    "UPDATE extract_jobs SET status='failed', finished_ts=?3, error=?4 WHERE file_uid=?1 AND content_hash=?2",
                    params![file_uid, content_hash, finished_ts, e.to_string()],
                );
                let _ = bus.publish_source(SourceEvent::ExtractionFailed {
                    file_uid: file_uid.clone(),
                    error: e.to_string(),
                });
            }
        }
    }
}

fn extract_one(
    _conn: &Connection,
    cfg: &Config,
    bus: &EventBus,
    file_uid: &str,
    content_hash: &str,
    path: &Utf8Path,
) -> Result<()> {
    let (extractor, extractor_version, pages) = extract_pages(path, cfg)?;
    bus.publish_source(SourceEvent::ExtractionCompleted {
        file_uid: file_uid.to_string(),
        content_hash: content_hash.to_string(),
        extractor,
        extractor_version,
        pages,
    })?;
    Ok(())
}

fn extract_pages(path: &Utf8Path, cfg: &Config) -> Result<(String, String, Vec<PageBlock>)> {
    let plain = is_plaintext(path);
    let text = if plain {
        fs::read_to_string(path).with_context(|| format!("read {path}"))?
    } else if cfg.extractor_cmd.trim().is_empty() {
        bail!("no extractor_cmd configured");
    } else {
        run_command(&cfg.extractor_cmd, path)?
    };
    let extractor = if plain {
        "builtin".to_string()
    } else {
        shell_words::split(&cfg.extractor_cmd)
            .ok()
            .and_then(|parts| parts.into_iter().next())
            .unwrap_or_else(|| "cmd".to_string())
    };
    let extractor_version = String::new();
    let pages = split_pages(&text);
    Ok((extractor, extractor_version, pages))
}

fn split_pages(text: &str) -> Vec<PageBlock> {
    let mut pages = Vec::new();
    let mut offset = 0usize;
    for (i, p) in text.split('\x0c').enumerate() {
        let len = p.chars().count();
        let start = offset;
        let end = start + len;
        pages.push(PageBlock {
            page_no: (i + 1) as u32,
            text: p.to_string(),
            start,
            end,
        });
        offset = end + 1; // account for the delimiter
    }
    pages
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

fn hash_file(path: &Utf8Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:016x}", hasher.digest()))
}

fn now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig};
    use std::sync::{atomic::AtomicBool, Arc};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn unicode_pages_preserved() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let file_path = root.join("u.txt");
        std::fs::write(&file_path, "αβγ\x0cδεζ")?;

        let cfg = crate::config::Config {
            db: root.join("catalog.db"),
            tantivy_index: Utf8PathBuf::from("idx"),
            roots: vec![root.clone()],
            include: vec!["**/*.txt".into()],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            include_hidden: false,
            allow_offline_hydration: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            extractor_cmd: String::new(),
            embedding: crate::config::EmbeddingConfig {
                provider: "disabled".into(),
            },
            mirror: MirrorConfig {
                root: Utf8PathBuf::from("raw"),
            },
            bus: BusConfig {
                bounds: BusBounds {
                    source_fs: 16,
                    mirror_text: 16,
                },
            },
            extract: ExtractConfig {
                pool_size: 1,
                jobs_bound: 16,
            },
        };

        let conn = db::open(&cfg.db)?;
        // Insert file metadata so worker can find path
        conn.execute(
            "INSERT INTO files (realpath, size, mtime_ns, fast_sig, is_offline, attrs, inode_hint, status, created_ts, updated_ts) VALUES (?1,0,0,'sig',0,0,?2,'active',0,0)",
            params![file_path.as_str(), "f1"],
        )?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(std::sync::Mutex::new(conn)));
        let rx = bus.subscribe_source();
        let stop = Arc::new(AtomicBool::new(false));
        let bus_run = bus.clone();
        let cfg_run = cfg.clone();
        let stop_run = stop.clone();
        std::thread::spawn(move || {
            run_pool(bus_run, &cfg_run, &stop_run).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));

        bus.publish_source(SourceEvent::ExtractionRequested {
            file_uid: "f1".into(),
        })?;

        use crossbeam_channel::RecvTimeoutError;
        let mut pages = None;
        for _ in 0..50 {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(env) => {
                    if let SourceEvent::ExtractionCompleted { pages: p, .. } = env.data {
                        pages = Some(p);
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        stop.store(true, Ordering::SeqCst);
        let pages = pages.expect("got pages");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].text, "αβγ");
        assert_eq!(pages[0].start, 0);
        assert_eq!(pages[0].end, 3);
        assert_eq!(pages[1].start, 4);
        assert_eq!(pages[1].end, 7);
        Ok(())
    }

    #[test]
    fn dedup_jobs() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let file_path = root.join("a.txt");
        std::fs::write(&file_path, "hello")?;

        let cfg = crate::config::Config {
            db: root.join("catalog.db"),
            tantivy_index: Utf8PathBuf::from("idx"),
            roots: vec![root.clone()],
            include: vec!["**/*.txt".into()],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            include_hidden: false,
            allow_offline_hydration: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            extractor_cmd: String::new(),
            embedding: crate::config::EmbeddingConfig {
                provider: "disabled".into(),
            },
            mirror: MirrorConfig {
                root: Utf8PathBuf::from("raw"),
            },
            bus: BusConfig {
                bounds: BusBounds {
                    source_fs: 16,
                    mirror_text: 16,
                },
            },
            extract: ExtractConfig {
                pool_size: 1,
                jobs_bound: 16,
            },
        };

        let conn = db::open(&cfg.db)?;
        conn.execute(
            "INSERT INTO files (realpath, size, mtime_ns, fast_sig, is_offline, attrs, inode_hint, status, created_ts, updated_ts) VALUES (?1,0,0,'sig',0,0,?2,'active',0,0)",
            params![file_path.as_str(), "f1"],
        )?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(std::sync::Mutex::new(conn)));
        let rx = bus.subscribe_source();
        let stop = Arc::new(AtomicBool::new(false));
        let bus_run = bus.clone();
        let cfg_run = cfg.clone();
        let stop_run = stop.clone();
        std::thread::spawn(move || {
            run_pool(bus_run, &cfg_run, &stop_run).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));

        bus.publish_source(SourceEvent::ExtractionRequested {
            file_uid: "f1".into(),
        })?;
        bus.publish_source(SourceEvent::ExtractionRequested {
            file_uid: "f1".into(),
        })?;

        let mut completed = 0;
        for _ in 0..50 {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(env) => {
                    if let SourceEvent::ExtractionCompleted { .. } = env.data {
                        completed += 1;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        stop.store(true, Ordering::SeqCst);
        assert_eq!(completed, 1);
        Ok(())
    }
}
