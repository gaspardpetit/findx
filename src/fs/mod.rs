use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use xxhash_rust::xxh3::Xxh3;

use crate::config::Config;
use crate::db;
use crate::extract;

/// Run a cold scan over all roots defined in configuration and update the DB.
pub fn cold_scan(cfg: &Config) -> Result<()> {
    let conn = db::open(&cfg.db)?;
    let include = build_glob_set(&cfg.include)?;
    let exclude = build_glob_set(&cfg.exclude)?;
    let mut seen: HashSet<Utf8PathBuf> = HashSet::new();

    for root in &cfg.roots {
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .follow_links(cfg.follow_symlinks)
            .build();
        for dent in walker {
            let dent = match dent {
                Ok(d) => d,
                Err(_) => continue,
            };
            if !dent.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let path = match Utf8Path::from_path(dent.path()) {
                Some(p) => p.to_owned(),
                None => continue,
            };
            if !include.is_match(path.as_std_path()) || exclude.is_match(path.as_std_path()) {
                continue;
            }
            process_path(&conn, &path, cfg)?;
            seen.insert(path);
        }
    }

    // mark deletions
    let mut stmt = conn.prepare("SELECT id, realpath FROM files WHERE status='active'")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let now_ts = now();
    for row in rows {
        let (id, rp) = row?;
        let p = Utf8PathBuf::from(rp.clone());
        if !seen.contains(&p) {
            conn.execute(
                "UPDATE files SET status='deleted', updated_ts=?2 WHERE id=?1",
                rusqlite::params![id, now_ts],
            )?;
            db::log_op(&conn, "del", Some(&rp), None, Some(id))?;
        }
    }
    Ok(())
}

fn process_path(conn: &rusqlite::Connection, path: &Utf8Path, cfg: &Config) -> Result<()> {
    use rusqlite::params;
    let meta = std::fs::metadata(path)?;
    let size = meta.len() as i64;
    let mtime = meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as i64;
    let hash = hash_file(path)?;
    let now_ts = now();

    let mut stmt = conn.prepare("SELECT id, size, mtime_ns, hash FROM files WHERE realpath=?1")?;
    let res = stmt.query_row(params![path.as_str()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    });
    let mut changed = false;
    let mut file_id: Option<i64> = None;
    match res {
        Ok((id, sz, mt, h)) => {
            if sz != size || mt != mtime || h != hash {
                conn.execute(
                    "UPDATE files SET size=?2, mtime_ns=?3, hash=?4, status='active', updated_ts=?5 WHERE id=?1",
                    params![id, size, mtime, hash, now_ts],
                )?;
                db::log_op(conn, "mod", Some(path.as_str()), None, Some(id))?;
                changed = true;
                file_id = Some(id);
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            conn.execute(
                "INSERT INTO files (realpath,size,mtime_ns,hash,created_ts,updated_ts) VALUES (?1,?2,?3,?4,?5,?5)",
                params![path.as_str(), size, mtime, hash, now_ts],
            )?;
            let id = conn.last_insert_rowid();
            db::log_op(conn, "add", None, Some(path.as_str()), Some(id))?;
            changed = true;
            file_id = Some(id);
        }
        Err(e) => return Err(e.into()),
    }
    if changed {
        if let Some(id) = file_id {
            let _ = extract::extract_file(conn, id, path, cfg);
        }
    }
    Ok(())
}

/// Watch for filesystem events and update the catalog.
pub fn watch(cfg: &Config) -> Result<()> {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let conn = db::open(&cfg.db)?;
    cold_scan(cfg)?;
    crate::index::reindex_all(cfg)?;

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, notify::Config::default())?;
    for root in &cfg.roots {
        watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;
    }

    let mut pending: HashMap<Utf8PathBuf, SystemTime> = HashMap::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if let Ok(p) = Utf8PathBuf::from_path_buf(path.clone()) {
                        pending.insert(p, SystemTime::now());
                    }
                }
            }
            Ok(Err(_)) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        let now = SystemTime::now();
        let to_process: Vec<_> = pending
            .iter()
            .filter(|(_, t)| {
                now.duration_since(**t).unwrap_or_default() > Duration::from_millis(300)
            })
            .map(|(p, _)| p.clone())
            .collect();
        for p in to_process {
            let _ = process_path(&conn, &p, cfg);
            pending.remove(&p);
        }
    }
    Ok(())
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    Ok(builder.build()?)
}

fn hash_file(path: &Utf8Path) -> Result<String> {
    let mut file = File::open(path)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn cold_scan_inserts_and_marks_deleted() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        std::fs::write(root.join("a.txt"), b"hello")?;
        let db_path = root.join("catalog.db");
        let cfg = Config {
            db: db_path.clone(),
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
        };

        cold_scan(&cfg)?;

        let conn = db::open(&db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status='active'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(count, 1);

        // delete file and rescan
        std::fs::remove_file(root.join("a.txt"))?;
        cold_scan(&cfg)?;
        let count_active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status='active'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(count_active, 0);
        let count_deleted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE status='deleted'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(count_deleted, 1);

        Ok(())
    }
}
