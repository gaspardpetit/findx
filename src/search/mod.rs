//! Query the Tantivy index for keyword search.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Value;
use tantivy::{Index, TantivyDocument};

use crate::config::Config;
use crate::index::{self, ChunkFields, IndexFields};
use crate::{db, embed};
use std::collections::HashMap;
use std::convert::TryInto;

#[derive(Serialize)]
pub struct SearchHit {
    pub path: String,
    pub score: f32,
    pub file_id: i64,
    pub mtime: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
}

#[derive(Serialize, Clone)]
pub struct ChunkSearchHit {
    pub path: String,
    pub score: f32,
    pub chunk_id: String,
    pub start_byte: i64,
    pub end_byte: i64,
}

#[derive(Serialize)]
pub struct ChunkSearchResults {
    pub results: Vec<ChunkSearchHit>,
}

/// Execute a keyword query against the index and return the top K results.
pub fn keyword(cfg: &Config, query: &str, top_k: usize) -> Result<SearchResults> {
    let index = Index::open_in_dir(cfg.tantivy_index.as_std_path())?;
    index::register_tokenizers(&index);
    let schema = index.schema();
    let fields = IndexFields::from_schema(&schema);
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let mut parser = QueryParser::for_index(&index, vec![fields.body_en, fields.body_fr]);
    parser.set_field_boost(fields.body_en, 1.0);
    parser.set_field_boost(fields.body_fr, 1.0);
    let q = parser.parse_query(query)?;
    let top_docs = searcher.search(&q, &TopDocs::with_limit(top_k))?;
    let mut hits = Vec::new();
    for (score, addr) in top_docs {
        let retrieved: TantivyDocument = searcher.doc(addr)?;
        let path = retrieved
            .get_first(fields.path)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let file_id = retrieved
            .get_first(fields.file_id)
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let mtime_ns = retrieved
            .get_first(fields.mtime_ns)
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let secs = mtime_ns / 1_000_000_000;
        let nanos = (mtime_ns % 1_000_000_000) as u32;
        let mtime = Utc
            .timestamp_opt(secs, nanos)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
        hits.push(SearchHit {
            path,
            score,
            file_id,
            mtime,
        });
    }
    Ok(SearchResults { results: hits })
}

/// Execute a keyword query against the chunk index and return the top K results.
pub fn keyword_chunks(cfg: &Config, query: &str, top_k: usize) -> Result<ChunkSearchResults> {
    let index_dir = cfg.tantivy_index.join("chunks");
    let index = Index::open_in_dir(index_dir.as_std_path())?;
    index::register_tokenizers(&index);
    let schema = index.schema();
    let fields = ChunkFields::from_schema(&schema);
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let mut parser =
        QueryParser::for_index(&index, vec![fields.chunk_text_en, fields.chunk_text_fr]);
    parser.set_field_boost(fields.chunk_text_en, 1.0);
    parser.set_field_boost(fields.chunk_text_fr, 1.0);
    let q = parser.parse_query(query)?;
    let top_docs = searcher.search(&q, &TopDocs::with_limit(top_k))?;
    let mut hits = Vec::new();
    for (score, addr) in top_docs {
        let retrieved: TantivyDocument = searcher.doc(addr)?;
        let path = retrieved
            .get_first(fields.path)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let chunk_id = retrieved
            .get_first(fields.chunk_id)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let start_byte = retrieved
            .get_first(fields.start_byte)
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let end_byte = retrieved
            .get_first(fields.end_byte)
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        hits.push(ChunkSearchHit {
            path,
            score,
            chunk_id,
            start_byte,
            end_byte,
        });
    }
    Ok(ChunkSearchResults { results: hits })
}

/// Execute a semantic query using embeddings over chunks.
pub fn semantic_chunks(cfg: &Config, query: &str, top_k: usize) -> Result<ChunkSearchResults> {
    let conn = db::open(&cfg.db)?;
    let q_vec = embed::embed_text(query)?;
    let mut stmt = conn.prepare(
        "SELECT e.chunk_id, e.vec, e.dim, f.realpath, c.start_byte, c.end_byte \
         FROM embeddings e JOIN chunks c ON e.chunk_id=c.chunk_id \
         JOIN files f ON f.id=c.file_id WHERE f.status='active' AND e.model_id='builtin'",
    )?;
    let rows = stmt.query_map([], |row| {
        let chunk_id: String = row.get(0)?;
        let vec_bytes: Vec<u8> = row.get(1)?;
        let dim: i64 = row.get(2)?;
        let path: String = row.get(3)?;
        let start_byte: i64 = row.get(4)?;
        let end_byte: i64 = row.get(5)?;
        let mut vec = Vec::with_capacity(dim as usize);
        for i in 0..dim as usize {
            let offset = i * 4;
            let arr: [u8; 4] = vec_bytes[offset..offset + 4].try_into().unwrap();
            vec.push(f32::from_le_bytes(arr));
        }
        Ok((chunk_id, vec, path, start_byte, end_byte))
    })?;
    let mut hits = Vec::new();
    for row in rows {
        let (chunk_id, vec, path, start_byte, end_byte) = row?;
        let score: f32 = q_vec.iter().zip(vec.iter()).map(|(a, b)| a * b).sum();
        hits.push(ChunkSearchHit {
            path,
            score,
            chunk_id,
            start_byte,
            end_byte,
        });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    hits.truncate(top_k);
    Ok(ChunkSearchResults { results: hits })
}

fn rrf(bm25: &[ChunkSearchHit], ann: &[ChunkSearchHit], top_k: usize) -> Vec<ChunkSearchHit> {
    let k_rrf = 60.0;
    let mut scores: HashMap<String, (ChunkSearchHit, f32)> = HashMap::new();
    for (rank, item) in bm25.iter().enumerate() {
        let contrib = 1.0 / (k_rrf + rank as f32 + 1.0);
        scores
            .entry(item.chunk_id.clone())
            .and_modify(|(_, s)| *s += contrib)
            .or_insert((item.clone(), contrib));
    }
    for (rank, item) in ann.iter().enumerate() {
        let contrib = 1.0 / (k_rrf + rank as f32 + 1.0);
        scores
            .entry(item.chunk_id.clone())
            .and_modify(|(_, s)| *s += contrib)
            .or_insert((item.clone(), contrib));
    }
    let mut out: Vec<ChunkSearchHit> = scores
        .into_iter()
        .map(|(_, (hit, s))| ChunkSearchHit { score: s, ..hit })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(top_k);
    out
}

/// Hybrid search combining BM25 and embedding scores with Reciprocal Rank Fusion.
pub fn hybrid_chunks(cfg: &Config, query: &str, top_k: usize) -> Result<ChunkSearchResults> {
    let bm25 = keyword_chunks(cfg, query, top_k)?.results;
    let ann = semantic_chunks(cfg, query, top_k)?.results;
    let fused = rrf(&bm25, &ann, top_k);
    Ok(ChunkSearchResults { results: fused })
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use crate::config::{Config, EmbeddingConfig};
    use crate::db;
    use rusqlite::params;

    #[test]
    fn keyword_search_returns_hit() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let db_path = root.join("catalog.db");
        let idx_path = root.join("idx");
        let cfg = Config {
            db: db_path.clone(),
            tantivy_index: idx_path.clone(),
            roots: vec![],
            include: vec![],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "en".into(),
            extractor_cmd: String::new(),
            embedding: EmbeddingConfig {
                provider: "disabled".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'','hello world',0,0)", [])?;

        index::reindex_all(&cfg, None)?;
        let res = keyword(&cfg, "hello", 10)?;
        assert_eq!(res.results.len(), 1);
        Ok(())
    }

    #[test]
    fn keyword_chunk_search_returns_hit() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let db_path = root.join("catalog.db");
        let idx_path = root.join("idx");
        let cfg = Config {
            db: db_path.clone(),
            tantivy_index: idx_path.clone(),
            roots: vec![],
            include: vec![],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "en".into(),
            extractor_cmd: String::new(),
            embedding: EmbeddingConfig {
                provider: "disabled".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        let long_text = "hello world".repeat(100);
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'',?1,0,0)",
            params![long_text])?;

        index::reindex_all(&cfg, None)?;
        let res = keyword_chunks(&cfg, "hello", 10)?;
        assert!(!res.results.is_empty());
        Ok(())
    }

    #[test]
    fn semantic_chunk_search_returns_hit() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let db_path = root.join("catalog.db");
        let idx_path = root.join("idx");
        let cfg = Config {
            db: db_path.clone(),
            tantivy_index: idx_path.clone(),
            roots: vec![],
            include: vec![],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "en".into(),
            extractor_cmd: String::new(),
            embedding: EmbeddingConfig {
                provider: "builtin".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        let long_text = "hello world".repeat(100);
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'',?1,0,0)", params![long_text])?;

        index::reindex_all(&cfg, None)?;
        let res = semantic_chunks(&cfg, "hello", 10)?;
        assert!(!res.results.is_empty());
        Ok(())
    }

    #[test]
    fn hybrid_chunk_search_returns_hit() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let db_path = root.join("catalog.db");
        let idx_path = root.join("idx");
        let cfg = Config {
            db: db_path.clone(),
            tantivy_index: idx_path.clone(),
            roots: vec![],
            include: vec![],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "en".into(),
            extractor_cmd: String::new(),
            embedding: EmbeddingConfig {
                provider: "builtin".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        let long_text = "hello world".repeat(100);
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'',?1,0,0)", params![long_text])?;

        index::reindex_all(&cfg, None)?;
        let res = hybrid_chunks(&cfg, "hello", 10)?;
        assert!(!res.results.is_empty());
        Ok(())
    }
}
