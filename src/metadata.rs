use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use camino::Utf8Path;
use rusqlite::params;

use crate::bus::EventBus;
use crate::config::Config;
use crate::db;
use crate::events::{FileMeta, FileMove, SourceEvent};

/// Run the metadata service, consuming `source.fs` events and updating the
/// `files` table. Added and modified files trigger `ExtractionRequested` events.
pub fn run(bus: EventBus, cfg: &Config) -> Result<()> {
    let conn = Arc::new(Mutex::new(db::open(&cfg.db)?));
    let rx = bus.subscribe_source();
    let publish_bus = bus.clone();
    drop(bus);
    while let Ok(env) = rx.recv() {
        match env.data {
            SourceEvent::SyncDelta {
                added,
                modified,
                moved,
                deleted,
            } => {
                handle_added(&publish_bus, &conn, &added)?;
                handle_modified(&publish_bus, &conn, &modified)?;
                handle_moved(&conn, &moved)?;
                handle_deleted(&conn, &deleted)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn handle_added(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    files: &[FileMeta],
) -> Result<()> {
    for f in files {
        let content_hash = hash_file(&f.path)?;
        let now_ts = now();
        let conn = conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO files (realpath, size, mtime_ns, hash, inode_hint, status, created_ts, updated_ts) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6)",
            params![f.path.as_str(), f.size as i64, f.mtime_ns, content_hash, f.file_uid, now_ts],
        )?;
        db::log_op(&conn, "add", None, Some(f.path.as_str()), None)?;
        drop(conn);
        bus.publish_source(SourceEvent::ExtractionRequested {
            file_uid: f.file_uid.clone(),
            content_hash,
        })?;
    }
    Ok(())
}

fn handle_modified(
    bus: &EventBus,
    conn: &Arc<Mutex<rusqlite::Connection>>,
    files: &[FileMeta],
) -> Result<()> {
    for f in files {
        let content_hash = hash_file(&f.path)?;
        let now_ts = now();
        let conn = conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET realpath=?2, size=?3, mtime_ns=?4, hash=?5, status='active', updated_ts=?6 WHERE inode_hint=?1",
            params![f.file_uid, f.path.as_str(), f.size as i64, f.mtime_ns, content_hash, now_ts],
        )?;
        db::log_op(&conn, "mod", Some(f.path.as_str()), None, None)?;
        drop(conn);
        bus.publish_source(SourceEvent::ExtractionRequested {
            file_uid: f.file_uid.clone(),
            content_hash,
        })?;
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

fn hash_file(path: &Utf8Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Xxh3::new();
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

use xxhash_rust::xxh3::Xxh3;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig};
    use camino::Utf8PathBuf;
    use std::sync::Arc;
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
            extract: ExtractConfig { pool_size: 1 },
        };

        let conn = db::open(&cfg.db)?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
        let bus_meta = bus.clone();
        let cfg_meta = cfg.clone();
        let handle = std::thread::spawn(move || {
            run(bus_meta, &cfg_meta).unwrap();
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

        drop(bus);
        handle.join().unwrap();
        Ok(())
    }
}
