use anyhow::Result;
use blake3::Hasher;
use rusqlite::{params, Connection};

/// Chunk all active documents in the database.
pub fn chunk_all(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.realpath, IFNULL(d.content_txt,'' ) FROM files f \
         JOIN documents d ON f.id=d.file_id WHERE f.status='active'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (file_id, path, content) = row?;
        chunk_document(conn, file_id, &path, &content)?;
    }
    Ok(())
}

fn chunk_document(conn: &Connection, file_id: i64, path: &str, content: &str) -> Result<()> {
    conn.execute("DELETE FROM chunks WHERE file_id=?1", params![file_id])?;
    let chunk_size = 2000; // bytes
    let overlap = 200; // bytes
    let mut start = 0;
    let len = content.len();
    while start < len {
        let mut end = std::cmp::min(start + chunk_size, len);
        while end < len && !content.is_char_boundary(end) {
            end += 1;
        }
        let text = &content[start..end];
        let token_count = text.split_whitespace().count() as i64;
        let mut hasher = Hasher::new();
        hasher.update(path.as_bytes());
        hasher.update(start.to_string().as_bytes());
        hasher.update(end.to_string().as_bytes());
        let chunk_id = hasher.finalize().to_hex().to_string();
        conn.execute(
            "INSERT INTO chunks (file_id, chunk_id, start_byte, end_byte, token_count, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![file_id, chunk_id, start as i64, end as i64, token_count, text],
        )?;
        if end == len {
            break;
        }
        start = end.saturating_sub(overlap);
        while start > 0 && !content.is_char_boundary(start) {
            start += 1;
        }
    }
    Ok(())
}
