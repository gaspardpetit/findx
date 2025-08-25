use std::fs;

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use rusqlite::{params, OptionalExtension};

use crate::bus::EventBus;
use crate::config::Config;
use crate::db;
use crate::events::{MirrorEvent, SourceEvent};

/// Reconcile the on-disk mirror with the `files` catalog.
///
/// For active files missing mirror artifacts or database entries, an
/// `ExtractionRequested` event is published so the extractor can rebuild the
/// mirror. Mirror entries whose source file is deleted result in removal of the
/// on-disk artifacts and a `MirrorDocDeleted` event.
pub fn run(bus: &EventBus, cfg: &Config) -> Result<()> {
    let conn = db::open(&cfg.db)?;

    {
        let mut stmt =
            conn.prepare("SELECT inode_hint, realpath FROM files WHERE status='active'")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (file_uid, path) = row?;
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM mirror_docs WHERE file_uid=?1",
                    params![&file_uid],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            let rel = relativize(Utf8Path::new(&path), &cfg.roots);
            let dir = cfg.mirror.root.join(&rel);
            let disk_exists = dir.join("meta.json").exists() && dir.join("chunks.jsonl").exists();
            if !exists || !disk_exists {
                bus.publish_source(SourceEvent::ExtractionRequested { file_uid })?;
            }
        }
    }

    {
        let mut stmt = conn.prepare("SELECT file_uid, path FROM mirror_docs")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (file_uid, relpath) = row?;
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM files WHERE inode_hint=?1",
                    params![&file_uid],
                    |r| r.get(0),
                )
                .optional()?;
            if status.as_deref() != Some("active") {
                let dir = cfg.mirror.root.join(&relpath);
                let _ = fs::remove_dir_all(&dir);
                conn.execute(
                    "DELETE FROM mirror_docs WHERE file_uid=?1",
                    params![&file_uid],
                )?;
                conn.execute(
                    "DELETE FROM mirror_chunks WHERE file_uid=?1",
                    params![&file_uid],
                )?;
                bus.publish_mirror(MirrorEvent::MirrorDocDeleted { file_uid })?;
            }
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::EventBus;
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig};
    use crossbeam_channel::RecvTimeoutError;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tempfile::tempdir;

    fn base_config(root: &Utf8Path) -> crate::config::Config {
        crate::config::Config {
            db: root.join("catalog.db"),
            tantivy_index: Utf8PathBuf::from("idx"),
            roots: vec![root.to_path_buf()],
            include: vec![],
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
                root: root.join("raw"),
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
        }
    }

    #[test]
    fn missing_mirror_triggers_extraction() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let cfg = base_config(&root);

        let conn = db::open(&cfg.db)?;
        conn.execute(
            "INSERT INTO files (realpath, size, mtime_ns, fast_sig, is_offline, attrs, inode_hint, status, created_ts, updated_ts) VALUES (?1, 0, 0, '', 0, 0, ?2, 'active', 0, 0)",
            params![root.join("a.txt").as_str(), "f1"],
        )?;

        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(db::open(&cfg.db)?)));
        let rx = bus.subscribe_source();
        run(&bus, &cfg)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(env) => match env.data {
                SourceEvent::ExtractionRequested { ref file_uid } => assert_eq!(file_uid, "f1"),
                _ => panic!("wrong event"),
            },
            Err(e) => match e {
                RecvTimeoutError::Timeout => panic!("no event"),
                RecvTimeoutError::Disconnected => panic!("disconnected"),
            },
        }
        Ok(())
    }

    #[test]
    fn removes_orphan_mirror() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let cfg = base_config(&root);

        let dir = cfg.mirror.root.join("b.txt");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("meta.json"), b"{}").unwrap();
        fs::write(dir.join("chunks.jsonl"), b"").unwrap();

        let conn = db::open(&cfg.db)?;
        conn.execute(
            "INSERT INTO mirror_docs (file_uid, content_hash, path, updated_ts) VALUES ('f2', 'h', 'b.txt', 0)",
            [],
        )?;

        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(db::open(&cfg.db)?)));
        let rx = bus.subscribe_mirror();
        run(&bus, &cfg)?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(env) => match env.data {
                MirrorEvent::MirrorDocDeleted { ref file_uid } => assert_eq!(file_uid, "f2"),
                _ => panic!("wrong event"),
            },
            Err(e) => match e {
                RecvTimeoutError::Timeout => panic!("no event"),
                RecvTimeoutError::Disconnected => panic!("disconnected"),
            },
        }
        let conn2 = db::open(&cfg.db)?;
        let count: i64 = conn2.query_row("SELECT COUNT(*) FROM mirror_docs", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        assert!(!dir.exists());
        Ok(())
    }
}
