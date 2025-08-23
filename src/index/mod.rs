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
use crate::db;

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
    Ok(())
}
