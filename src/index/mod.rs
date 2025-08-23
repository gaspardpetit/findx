//! Tantivy index builder for `localindex`.

use std::fs;

use anyhow::Result;
use camino::Utf8Path;
use tantivy::schema::{
    Field, Schema, SchemaBuilder, TextFieldIndexing, TextOptions, STORED, STRING,
};
use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer};
use tantivy::{doc, Index};

use crate::config::Config;
use crate::{chunk, db};
use rusqlite::params;

/// Fields used in the Tantivy schema.
#[derive(Clone, Copy)]
pub struct IndexFields {
    pub path: Field,
    pub body_en: Field,
    pub body_fr: Field,
    pub mime: Field,
    pub mtime_ns: Field,
    pub size: Field,
    pub file_id: Field,
}

impl IndexFields {
    /// Build a `IndexFields` from an existing schema.
    pub fn from_schema(schema: &Schema) -> Self {
        Self {
            path: schema.get_field("path").unwrap(),
            body_en: schema.get_field("body_en").unwrap(),
            body_fr: schema.get_field("body_fr").unwrap(),
            mime: schema.get_field("mime").unwrap(),
            mtime_ns: schema.get_field("mtime_ns").unwrap(),
            size: schema.get_field("size").unwrap(),
            file_id: schema.get_field("file_id").unwrap(),
        }
    }
}

fn build_schema() -> (Schema, IndexFields) {
    let mut builder = SchemaBuilder::new();
    let path = builder.add_text_field("path", STRING | STORED);
    let en_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let fr_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("fr")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let body_en = builder.add_text_field("body_en", en_opts);
    let body_fr = builder.add_text_field("body_fr", fr_opts);
    let mime = builder.add_text_field("mime", STRING | STORED);
    let mtime_ns = builder.add_i64_field("mtime_ns", STORED);
    let size = builder.add_i64_field("size", STORED);
    let file_id = builder.add_i64_field("file_id", STORED);
    let schema = builder.build();
    (
        schema.clone(),
        IndexFields {
            path,
            body_en,
            body_fr,
            mime,
            mtime_ns,
            size,
            file_id,
        },
    )
}

/// Fields used for the chunk-level Tantivy schema.
#[derive(Clone, Copy)]
pub struct ChunkFields {
    pub path: Field,
    pub chunk_text_en: Field,
    pub chunk_text_fr: Field,
    pub chunk_id: Field,
    pub start_byte: Field,
    pub end_byte: Field,
    pub file_id: Field,
}

impl ChunkFields {
    pub fn from_schema(schema: &Schema) -> Self {
        Self {
            path: schema.get_field("path").unwrap(),
            chunk_text_en: schema.get_field("chunk_text_en").unwrap(),
            chunk_text_fr: schema.get_field("chunk_text_fr").unwrap(),
            chunk_id: schema.get_field("chunk_id").unwrap(),
            start_byte: schema.get_field("start_byte").unwrap(),
            end_byte: schema.get_field("end_byte").unwrap(),
            file_id: schema.get_field("file_id").unwrap(),
        }
    }
}

fn build_chunk_schema() -> (Schema, ChunkFields) {
    let mut builder = SchemaBuilder::new();
    let path = builder.add_text_field("path", STRING | STORED);
    let en_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("en")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let fr_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("fr")
            .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
    );
    let chunk_text_en = builder.add_text_field("chunk_text_en", en_opts);
    let chunk_text_fr = builder.add_text_field("chunk_text_fr", fr_opts);
    let chunk_id = builder.add_text_field("chunk_id", STRING | STORED);
    let start_byte = builder.add_i64_field("start_byte", STORED);
    let end_byte = builder.add_i64_field("end_byte", STORED);
    let file_id = builder.add_i64_field("file_id", STORED);
    let schema = builder.build();
    (
        schema.clone(),
        ChunkFields {
            path,
            chunk_text_en,
            chunk_text_fr,
            chunk_id,
            start_byte,
            end_byte,
            file_id,
        },
    )
}

pub fn register_tokenizers(index: &Index) {
    let manager = index.tokenizers();
    let en = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveLongFilter::limit(40))
        .build();
    manager.register("en", en);
    let fr = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(RemoveLongFilter::limit(40))
        .build();
    manager.register("fr", fr);
}

/// Rebuild the entire Tantivy index from the SQLite catalog.
pub fn reindex_all(cfg: &Config) -> Result<()> {
    let conn = db::open(&cfg.db)?;
    let index_dir: &Utf8Path = &cfg.tantivy_index;
    if index_dir.exists() {
        fs::remove_dir_all(index_dir)?;
    }
    fs::create_dir_all(index_dir)?;
    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(index_dir.as_std_path(), schema)?;
    register_tokenizers(&index);
    let mut writer = index.writer(50_000_000)?; // 50MB

    let mut stmt = conn.prepare(
        "SELECT f.id, f.realpath, f.mtime_ns, f.size, IFNULL(f.mime, ''), \
                 IFNULL(d.lang, ''), IFNULL(d.content_txt, '') \
         FROM files f JOIN documents d ON f.id=d.file_id \
         WHERE f.status='active'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    for row in rows {
        let (id, path, mtime_ns, size, mime, lang, content) = row?;
        let mut tdoc = doc!(
            fields.path => path.clone(),
            fields.mime => mime,
            fields.mtime_ns => mtime_ns,
            fields.size => size,
            fields.file_id => id,
        );
        match lang.as_str() {
            "en" => tdoc.add_text(fields.body_en, &content),
            "fr" => tdoc.add_text(fields.body_fr, &content),
            _ => {
                tdoc.add_text(fields.body_en, &content);
                tdoc.add_text(fields.body_fr, &content);
            }
        }
        writer.add_document(tdoc)?;
    }

    writer.commit()?;

    // Chunk documents and build chunk index
    chunk::chunk_all(&conn)?;
    let chunk_dir = index_dir.join("chunks");
    if chunk_dir.exists() {
        fs::remove_dir_all(&chunk_dir)?;
    }
    fs::create_dir_all(&chunk_dir)?;
    let (chunk_schema, chunk_fields) = build_chunk_schema();
    let chunk_index = Index::create_in_dir(chunk_dir.as_std_path(), chunk_schema)?;
    register_tokenizers(&chunk_index);
    let mut chunk_writer = chunk_index.writer(50_000_000)?;

    let mut stmt = conn.prepare(
        "SELECT f.id, f.realpath, IFNULL(d.lang,''), c.chunk_id, c.start_byte, c.end_byte, c.text \
         FROM chunks c JOIN files f ON f.id=c.file_id \
         JOIN documents d ON d.file_id=f.id \
         WHERE f.status='active'",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    for row in rows {
        let (file_id, path, lang, chunk_id, start_byte, end_byte, text) = row?;
        let mut tdoc = doc!(
            chunk_fields.path => path,
            chunk_fields.chunk_id => chunk_id,
            chunk_fields.start_byte => start_byte,
            chunk_fields.end_byte => end_byte,
            chunk_fields.file_id => file_id,
        );
        match lang.as_str() {
            "en" => tdoc.add_text(chunk_fields.chunk_text_en, &text),
            "fr" => tdoc.add_text(chunk_fields.chunk_text_fr, &text),
            _ => {
                tdoc.add_text(chunk_fields.chunk_text_en, &text);
                tdoc.add_text(chunk_fields.chunk_text_fr, &text);
            }
        }
        chunk_writer.add_document(tdoc)?;
    }

    chunk_writer.commit()?;

    // Compute embeddings for chunks if enabled
    if cfg.embedding.provider != "disabled" {
        let mut stmt = conn.prepare("SELECT chunk_id, text FROM chunks")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (chunk_id, text) = row?;
            let emb = crate::embed::embed_text(&text)?;
            let vec_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            conn.execute(
                "INSERT OR REPLACE INTO embeddings (chunk_id, model_id, dim, vec) VALUES (?1, ?2, ?3, ?4)",
                params![chunk_id, "builtin", emb.len() as i64, vec_bytes],
            )?;
        }
    }
    Ok(())
}
