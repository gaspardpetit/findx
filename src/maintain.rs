use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{params, Connection};

use crate::config::Config;
use crate::db;

/// Run database retention tasks according to configuration.
pub fn run(cfg: &Config) -> Result<()> {
    let conn = db::open(&cfg.db)?;
    let now = now();
    prune_events(&conn, now, cfg.retention.events_days)?;
    prune_extract_jobs(
        &conn,
        now,
        cfg.retention.jobs_keep_per_file,
        cfg.retention.jobs_failed_days,
    )?;
    prune_files(&conn, now, cfg.retention.files_tombstone_days)?;
    clean_orphans(&conn, cfg)?;
    vacuum_if_needed(&conn)?;
    Ok(())
}

fn prune_events(conn: &Connection, now: i64, days: u64) -> Result<()> {
    let cutoff = now - (days as i64) * 86_400;
    conn.execute("DELETE FROM events WHERE ts < ?1", params![cutoff])?;
    Ok(())
}

fn prune_extract_jobs(
    conn: &Connection,
    now: i64,
    keep_per_file: usize,
    failed_days: u64,
) -> Result<()> {
    let cutoff_failed = now - (failed_days as i64) * 86_400;
    conn.execute(
        "DELETE FROM extract_jobs WHERE status='failed' AND finished_ts IS NOT NULL AND finished_ts < ?1",
        params![cutoff_failed],
    )?;
    conn.execute(
        "DELETE FROM extract_jobs WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (PARTITION BY file_uid ORDER BY id DESC) AS rn
                FROM extract_jobs
            ) WHERE rn > ?1
        )",
        params![keep_per_file],
    )?;
    Ok(())
}

fn prune_files(conn: &Connection, now: i64, days: u64) -> Result<()> {
    let cutoff = now - (days as i64) * 86_400;
    conn.execute(
        "DELETE FROM files WHERE status!='active' AND updated_ts < ?1",
        params![cutoff],
    )?;
    Ok(())
}

fn clean_orphans(conn: &Connection, cfg: &Config) -> Result<()> {
    // Remove mirror artifacts whose source file no longer exists.
    let mut stmt = conn.prepare(
        "SELECT file_uid, path FROM mirror_docs WHERE file_uid NOT IN (SELECT inode_hint FROM files)",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (uid, path) = row?;
        let dir = cfg.mirror.root.join(&path);
        let _ = fs::remove_dir_all(&dir);
        conn.execute("DELETE FROM mirror_chunks WHERE file_uid=?1", params![&uid])?;
        conn.execute("DELETE FROM mirror_docs WHERE file_uid=?1", params![&uid])?;
    }
    // Remove mirror chunks without parent docs.
    conn.execute(
        "DELETE FROM mirror_chunks WHERE file_uid NOT IN (SELECT file_uid FROM mirror_docs)",
        [],
    )?;
    Ok(())
}

fn vacuum_if_needed(conn: &Connection) -> Result<()> {
    let page_count: i64 = conn.query_row("PRAGMA page_count;", [], |r| r.get(0))?;
    let free: i64 = conn.query_row("PRAGMA freelist_count;", [], |r| r.get(0))?;
    if free > 1000 && free * 10 > page_count {
        conn.execute_batch("VACUUM;")?;
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
    use crate::config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig, RetentionConfig};
    use tempfile::tempdir;

    fn base_config(root: &camino::Utf8Path) -> Config {
        Config {
            db: root.join("catalog.db"),
            tantivy_index: camino::Utf8PathBuf::from("idx"),
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
            retention: RetentionConfig::default(),
        }
    }

    #[test]
    fn prunes_old_rows() -> Result<()> {
        let tmp = tempdir()?;
        let root = camino::Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let mut cfg = base_config(&root);
        cfg.retention.jobs_keep_per_file = 1;
        fs::create_dir_all(&cfg.mirror.root)?;
        let conn = db::open(&cfg.db)?;
        // old event
        conn.execute(
            "INSERT INTO events (ts, topic, type, idempotency_key, payload) VALUES (0,'t','t','k','{}')",
            [],
        )?;
        // new event
        conn.execute(
            "INSERT INTO events (ts, topic, type, idempotency_key, payload) VALUES (?1,'t','t','k2','{}')",
            params![now()],
        )?;
        // extract jobs
        conn.execute(
            "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts, finished_ts) VALUES ('f1','h','failed',0,0,0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts, finished_ts) VALUES ('f1','h2','done',0,0,0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts, finished_ts) VALUES ('f1','h3','done',0,0,0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO extract_jobs (file_uid, content_hash, status, attempt, started_ts, finished_ts) VALUES ('f1','h4','done',0,0,0)",
            [],
        )?;
        // tombstoned file
        conn.execute(
            "INSERT INTO files (realpath,size,mtime_ns,fast_sig,is_offline,attrs,inode_hint,status,created_ts,updated_ts) VALUES ('a',0,0,'',0,0,'uid1','deleted',0,0)",
            [],
        )?;
        // orphan mirror doc
        let dir = cfg.mirror.root.join("a");
        fs::create_dir_all(&dir)?;
        conn.execute(
            "INSERT INTO mirror_docs (file_uid, content_hash, path, updated_ts) VALUES ('uid2','h','a',0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO mirror_chunks (chunk_id, file_uid, ord) VALUES ('c1','uid2',0)",
            [],
        )?;
        drop(conn);
        run(&cfg)?;
        let conn = db::open(&cfg.db)?;
        let ev_count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))?;
        assert_eq!(ev_count, 1);
        let job_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM extract_jobs", [], |r| r.get(0))?;
        assert_eq!(job_count, 1);
        let file_count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        assert_eq!(file_count, 0);
        let md_count: i64 = conn.query_row("SELECT COUNT(*) FROM mirror_docs", [], |r| r.get(0))?;
        assert_eq!(md_count, 0);
        assert!(!dir.exists());
        Ok(())
    }
}
