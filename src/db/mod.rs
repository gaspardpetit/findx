use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use camino::Utf8Path;
use rusqlite::{params, Connection};

/// Open a connection to the SQLite database at `path` and ensure the schema.
pub fn open(path: &Utf8Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create db parent dir {parent}"))?;
    }
    tracing::debug!(%path, "opening database");
    let conn = Connection::open(path.as_str()).with_context(|| format!("open db at {path}"))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS files (
          id INTEGER PRIMARY KEY,
          realpath TEXT UNIQUE NOT NULL,
          size INTEGER NOT NULL,
          mtime_ns INTEGER NOT NULL,
          inode_hint TEXT,
          mime TEXT,
          hash TEXT,
          status TEXT NOT NULL DEFAULT 'active',
          created_ts INTEGER NOT NULL,
          updated_ts INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ops_log (
          ts INTEGER NOT NULL,
          kind TEXT NOT NULL,
          path_from TEXT,
          path_to TEXT,
          file_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS documents (
          file_id INTEGER PRIMARY KEY,
          extractor TEXT NOT NULL,
          extractor_version TEXT NOT NULL,
          lang TEXT,
          page_count INTEGER,
          content_md BLOB,
          content_txt BLOB,
          ocr_applied INTEGER NOT NULL DEFAULT 0,
          updated_ts INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS chunks (
          file_id INTEGER NOT NULL,
          chunk_id TEXT PRIMARY KEY,
          start_byte INTEGER NOT NULL,
          end_byte INTEGER NOT NULL,
          page_from INTEGER,
          page_to INTEGER,
          section_path TEXT,
          token_count INTEGER,
          text BLOB NOT NULL
        );
        CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id);
        CREATE TABLE IF NOT EXISTS embeddings (
          chunk_id TEXT NOT NULL,
          model_id TEXT NOT NULL,
          dim INTEGER NOT NULL,
          vec BLOB NOT NULL,
          PRIMARY KEY(chunk_id, model_id)
        );
        CREATE TABLE IF NOT EXISTS events (
          id INTEGER PRIMARY KEY,
          ts INTEGER NOT NULL,
          topic TEXT NOT NULL,
          type TEXT NOT NULL,
          idempotency_key TEXT NOT NULL,
          payload TEXT NOT NULL
        );
        "#,
    )?;
    Ok(conn)
}

/// Insert a record into `ops_log`.
pub fn log_op(
    conn: &Connection,
    kind: &str,
    path_from: Option<&str>,
    path_to: Option<&str>,
    file_id: Option<i64>,
) -> Result<()> {
    let ts = now();
    conn.execute(
        "INSERT INTO ops_log (ts, kind, path_from, path_to, file_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![ts, kind, path_from, path_to, file_id],
    )?;
    Ok(())
}

/// Return current unix timestamp seconds.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
