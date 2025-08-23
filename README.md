[![Rust](https://github.com/gaspardpetit/localindex/actions/workflows/rust.yml/badge.svg)](https://github.com/gaspardpetit/localindex/actions/workflows/rust.yml)
# localindex

`localindex` is a Rust CLI for indexing and searching local documents. It scans files under configured roots, extracts textual content, and builds a searchable [Tantivy](https://tantivy-search.github.io/) index.

## Layout

```
localindex/
  Cargo.toml
  src/
    main.rs
    cli.rs
    config.rs
    db/
    fs/
    extract/
    index/
    search/
    util/
  tools/
  docs/
  examples/
```

## Configuration

The application reads settings from a TOML file. A sample `localindex.toml` is provided:

```toml
db = "state/catalog.db"
tantivy_index = "state/idx"
roots = ["/data/a"]
include = ["**/*.pdf", "**/*.docx", "**/*.md", "**/*.txt"]
exclude = ["**/.git/**", "**/~$*"]
max_file_size_mb = 200
follow_symlinks = false
commit_interval_secs = 45
guard_interval_secs = 180
default_language = "auto"
extractor_url = "http://127.0.0.1:8878/extract"

[embedding]
provider = "disabled"
```

## Filesystem cataloging

The `index` command performs a cold scan of the configured roots and
stores file metadata in a SQLite database (`files` and `ops_log` tables).
The `watch` command runs the scan and then watches for filesystem
changes, updating the catalog as files are added, modified, or deleted.

## Content extraction

During indexing, `localindex` calls a Python sidecar to extract text and
Markdown from documents. Results are stored in a `documents` table with
metadata such as language and page counts. The sidecar endpoint is
configured via `extractor_url`.

## Keyword search

After a scan completes, `localindex` builds a BM25 index using Tantivy.
Documents are indexed into language-specific fields (`body_en`, `body_fr`) based on the
detected language. Keyword queries return the top matches with scores and metadata:

```bash
localindex query --tantivy-index state/idx --db state/catalog.db \
  --mode keyword --top-k 20 "indemnity carve-out"
```

Example JSON output:

```json
{"results":[{"path":"/data/a/msa.pdf","score":12.3,"file_id":42,"mtime":"2025-07-05T12:43:11Z"}]}
```

## Chunking and chunk search

During indexing, documents are split into overlapping chunks which are stored in a `chunks`
table and indexed separately under `tantivy_index/chunks`. Queries can target chunks instead
of whole documents by passing `--chunks`:

```bash
localindex query --tantivy-index state/idx --db state/catalog.db \
  --mode keyword --chunks "résiliation pour faute grave"
```

Example chunk result:

```json
{"results":[{"path":"/data/a/msa.pdf","score":9.8,"chunk_id":"abcd..","start_byte":182340,"end_byte":183912}]}
```

## Embeddings and semantic search

Chunks can be embedded into vectors for multilingual semantic search. When the
embedding provider is enabled (`embedding.provider = "builtin"`), each chunk is
encoded and stored in an `embeddings` table.

Semantic search queries the stored vectors directly:

```bash
localindex query --tantivy-index state/idx --db state/catalog.db \
  --mode semantic "How long do we store subcontractor data?"
```

Hybrid search combines BM25 and semantic scores using reciprocal rank fusion:

```bash
localindex query --tantivy-index state/idx --db state/catalog.db \
  --mode hybrid "indemnité plafond carve-out"
```

## Building

```bash
cargo build
```

## Releases

Prebuilt binaries for Linux, macOS, and Windows are available on the [GitHub Releases](https://github.com/gaspardpetit/localindex/releases) page.
These binaries embed the release tag; verify with `localindex --version`.

Snapshot artifacts for the `main` branch are published by the `snapshot` workflow.

## Help

```bash
cargo run -- --help
```
