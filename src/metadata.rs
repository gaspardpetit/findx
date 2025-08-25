use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::params;

use crate::bus::EventBus;
use crate::config::Config;
use crate::db;
use crate::events::{FileMeta, FileMove, SourceEvent};
use crossbeam_channel::RecvTimeoutError;

/// Run the metadata service, consuming `source.fs` events and updating the
/// `files` table. Added and modified files trigger `ExtractionRequested` events.
pub fn run(bus: EventBus, cfg: &Config, stop: &AtomicBool) -> Result<()> {
    let conn = Arc::new(Mutex::new(db::open(&cfg.db)?));
    let rx = bus.subscribe_source();
    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(env) => match env.data {
                SourceEvent::SyncDelta {
                    added,
                    modified,
                    moved,
                    deleted,
                } => {
                    handle_added(&bus, &conn, cfg, &added)?;
                    handle_modified(&bus, &conn, cfg, &modified)?;
                    handle_moved(&conn, &moved)?;
                    handle_deleted(&conn, &deleted)?;
                }
                _ => {}
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn handle_added(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    cfg: &Config,
    files: &[FileMeta],
) -> Result<()> {
    for f in files {
        let now_ts = now();
        let conn = conn.lock().unwrap();
        let status = if f.is_offline { "offline" } else { "active" };
        conn.execute(
            "INSERT OR REPLACE INTO files (realpath, size, mtime_ns, fast_sig, is_offline, attrs, inode_hint, status, created_ts, updated_ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                f.path.as_str(),
                f.size as i64,
                f.mtime_ns,
                f.fast_sig,
                f.is_offline as i64,
                f.attrs as i64,
                f.file_uid,
                status,
                now_ts
            ],
        )?;
        db::log_op(&conn, "add", None, Some(f.path.as_str()), None)?;
        drop(conn);
        if !f.is_offline || cfg.allow_offline_hydration {
            bus.publish_source(SourceEvent::ExtractionRequested {
                file_uid: f.file_uid.clone(),
            })?;
        }
    }
    Ok(())
}

fn handle_modified(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    cfg: &Config,
    files: &[FileMeta],
) -> Result<()> {
    for f in files {
        let now_ts = now();
        let conn = conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET realpath=?2, size=?3, mtime_ns=?4, fast_sig=?5, is_offline=?6, attrs=?7, hash=NULL, status='active', updated_ts=?8 WHERE inode_hint=?1",
            params![
                f.file_uid,
                f.path.as_str(),
                f.size as i64,
                f.mtime_ns,
                f.fast_sig,
                f.is_offline as i64,
                f.attrs as i64,
                now_ts
            ],
        )?;
        db::log_op(&conn, "mod", Some(f.path.as_str()), None, None)?;
        drop(conn);
        if !f.is_offline || cfg.allow_offline_hydration {
            bus.publish_source(SourceEvent::ExtractionRequested {
                file_uid: f.file_uid.clone(),
            })?;
        }
    }
    Ok(())
}

fn handle_moved(conn: &Arc<Mutex<rusqlite::Connection>>, moves: &[FileMove]) -> Result<()> {
    for m in moves {
        let now_ts = now();
        let conn = conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET realpath=?2, updated_ts=?3 WHERE inode_hint=?1",
            params![m.file_uid, m.to.as_str(), now_ts],
        )?;
        db::log_op(
            &conn,
            "mv",
            Some(m.from.as_str()),
            Some(m.to.as_str()),
            None,
        )?;
    }
    Ok(())
}

fn handle_deleted(conn: &Arc<Mutex<rusqlite::Connection>>, files: &[FileMeta]) -> Result<()> {
    for f in files {
        let now_ts = now();
        let conn = conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET status='deleted', updated_ts=?2 WHERE inode_hint=?1",
            params![f.file_uid, now_ts],
        )?;
        db::log_op(&conn, "del", Some(f.path.as_str()), None, None)?;
    }
    Ok(())
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
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig, RetentionConfig};
    use camino::Utf8PathBuf;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn move_preserves_file_uid() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        std::fs::write(root.join("a.txt"), b"hello")?;

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
            retention: RetentionConfig::default(),
        };

        let conn = db::open(&cfg.db)?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
        let bus_meta = bus.clone();
        let cfg_meta = cfg.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_run = stop.clone();
        let handle = std::thread::spawn(move || {
            run(bus_meta, &cfg_meta, &stop_run).unwrap();
        });

        let mut state = crate::fs::FsState::default();
        crate::fs::cold_scan(&cfg, &bus, &mut state)?;
        std::thread::sleep(Duration::from_millis(200));

        let conn = db::open(&cfg.db)?;
        let uid: String = conn.query_row(
            "SELECT inode_hint FROM files WHERE status='active'",
            [],
            |r| r.get(0),
        )?;

        std::fs::rename(root.join("a.txt"), root.join("b.txt"))?;
        crate::fs::cold_scan(&cfg, &bus, &mut state)?;
        std::thread::sleep(Duration::from_millis(200));

        let (uid2, path): (String, String) = conn.query_row(
            "SELECT inode_hint, realpath FROM files WHERE status='active'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

        assert_eq!(uid, uid2);
        assert!(path.ends_with("b.txt"));

        stop.store(true, Ordering::SeqCst);
        drop(bus);
        handle.join().unwrap();
        Ok(())
    }
}
