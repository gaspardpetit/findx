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

#[derive(Serialize)]
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
            extractor_url: String::new(),
            embedding: EmbeddingConfig {
                provider: "disabled".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'','hello world',0,0)", [])?;

        index::reindex_all(&cfg)?;
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
            extractor_url: String::new(),
            embedding: EmbeddingConfig {
                provider: "disabled".into(),
            },
        };

        let conn = db::open(&db_path)?;
        conn.execute("INSERT INTO files (id, realpath, size, mtime_ns, status, created_ts, updated_ts) VALUES (1,'/tmp/a.txt',1,0,'active',0,0)", [])?;
        let long_text = "hello world".repeat(100);
        conn.execute("INSERT INTO documents (file_id, extractor, extractor_version, lang, page_count, content_md, content_txt, ocr_applied, updated_ts) VALUES (1,'doc','v','en',1,'',?1,0,0)",
            params![long_text])?;

        index::reindex_all(&cfg)?;
        let res = keyword_chunks(&cfg, "hello", 10)?;
        assert!(!res.results.is_empty());
        Ok(())
    }
}
