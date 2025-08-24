[![Rust](https://github.com/gaspardpetit/findx/actions/workflows/rust.yml/badge.svg)](https://github.com/gaspardpetit/findx/actions/workflows/rust.yml)
# findx

`findx` is a Rust CLI for indexing and searching local documents. It scans files under configured roots, extracts textual content, and builds a searchable [Tantivy](https://tantivy-search.github.io/) index.

## Quick start

```bash
findx index
findx query rust documentation
```
By default, query results are printed as human-readable JSON. Pass `--compact-output` or set `COMPACT_OUTPUT=1` for single-line output.

The commands above index the current directory and place all data under `.findx/`, creating the directory if it does not exist. Runtime state such as the lockfile lives under `.findx/state`. Query defaults to a hybrid search mode.

## Dependencies

A complete list of build and run-time dependencies is available in [doc/dependencies.md](doc/dependencies.md).

## Layout

```
findx/
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

The application reads settings from a TOML file. A sample `findx.toml` is provided:

```toml
db = ".findx/catalog.db"
tantivy_index = ".findx/idx"
roots = ["."]
include = ["**/*.pdf", "**/*.docx", "**/*.md", "**/*.txt"]
exclude = ["**/.git/**", "**/~$*"]
max_file_size_mb = 200
follow_symlinks = false
commit_interval_secs = 45
guard_interval_secs = 180
default_language = "auto"
extractor_cmd = "docling --to text"

[embedding]
provider = "disabled"
```

## Filesystem cataloging

The `index` command performs a cold scan of the configured roots and
stores file metadata in a SQLite database (`files` and `ops_log` tables).
The `watch` command runs the scan and then watches for filesystem
changes, updating the catalog as files are added, modified, or deleted.
It listens for `SIGINT` and `SIGTERM` to shut down cleanly.

During indexing, a textual dashboard shows progress for files and chunks
when running in a terminal, including the path of the file currently
being processed. The dashboard is suppressed in non-console contexts.
Set `LOG_LEVEL` (e.g. `debug`, `info`) to control log verbosity. Each
run appends a plain text log to `.findx/index.log` with file statuses,
chunk counts, and the final Tantivy index size.
Transient `PermissionDenied` errors (such as antivirus scans locking
Tantivy files on Windows) are detected and the index build automatically
retries a few times.

## Content extraction

During indexing, `findx` converts documents to plain text using a
configurable command (`extractor_cmd`). By default it invokes the
[`docling`](https://github.com/docling) CLI with `--to text`. Basic text
formats like `.txt` or `.md` are read directly without invoking an
external tool.
Results are stored in a `documents` table with metadata such as language
and page counts.

## Keyword search

After a scan completes, `findx` builds a BM25 index using Tantivy.
Documents are indexed into language-specific fields (`body_en`, `body_fr`) based on the
detected language. Tokenization preserves decimals and dotted acronyms so references like
`12.4.1` or `C.c.Q.` remain searchable as single terms. Keyword queries return the top
matches with scores and metadata:

```bash
findx query --tantivy-index .findx/idx --db .findx/catalog.db \
  --mode keyword --top-k 20 "project timeline"
```

Example JSON output:

```json
{
  "results": [
    {
      "path": "./design_spec.pdf",
      "score": 12.3,
      "file_id": 42,
      "mtime": "2025-07-05T12:43:11Z"
    }
  ]
}
```

## Chunking and chunk search

During indexing, documents are split into overlapping chunks which are stored in a `chunks`
table and indexed separately under `tantivy_index/chunks`. Queries can target chunks instead
of whole documents by passing `--chunks`:

```bash
findx query --tantivy-index .findx/idx --db .findx/catalog.db \
  --mode keyword --chunks "project kickoff agenda"
```

Example chunk result:

```json
{
  "results": [
    {
      "path": "./design_spec.pdf",
      "score": 9.8,
      "chunk_id": "abcd..",
      "start_byte": 182340,
      "end_byte": 183912
    }
  ]
}
```

## Embeddings and semantic search

Chunks can be embedded into vectors for multilingual semantic search. When the
embedding provider is enabled (`embedding.provider = "builtin"`), each chunk is
  encoded and stored in an `embeddings` table. By default `findx` uses a
  Rust native embedder powered by [fastembed](https://crates.io/crates/fastembed),
  caching models under `.findx/fastembed_cache`. It downloads a supported model
  the first time it runs. You can hint another
  model by setting `EMBEDDING_MODEL` to a name from
  `TextEmbedding::list_supported_models()`. If the requested model is unsupported
  or cannot be downloaded, `findx` returns an error instead of falling back
to a default embedding model.

Before attempting a network download, `findx` looks for model files under
`models/<model_name>/`. Supplying an ONNX model and tokenizer files in this
directory lets you run entirely offline. For example, to use the small
`snowflake/snowflake-arctic-embed-xs` model in tests, place its
`model_uint8.onnx`, `tokenizer.json`, `config.json`, `tokenizer_config.json`,
and `special_tokens_map.json` under
`models/snowflake/snowflake-arctic-embed-xs/` and set
`EMBEDDING_MODEL=snowflake/snowflake-arctic-embed-xs`.

To use an external embedding service instead, set `EMBEDDING_URL` (and
optionally `EMBEDDING_API_KEY`). Any value in `EMBEDDING_MODEL` will be forwarded
in the request payload for provider-specific model selection.

Semantic search queries the stored vectors directly:

```bash
findx query --tantivy-index .findx/idx --db .findx/catalog.db \
  --mode semantic "How do we set up continuous integration?"
```

Hybrid search combines BM25 and semantic scores using reciprocal rank fusion:

```bash
findx query --tantivy-index .findx/idx --db .findx/catalog.db \
  --mode hybrid "performance optimization techniques"
```

## Building

Requires Rust 1.88 or newer.

```bash
cargo build
```

## Releases

Prebuilt binaries for Linux, macOS, and Windows are available on the [GitHub Releases](https://github.com/gaspardpetit/findx/releases) page.
These binaries embed the release tag; verify with `findx --version`.

Snapshot artifacts for the `main` branch are published by the `snapshot` workflow.

## Docker

A published container image can run `findx` against a mounted directory. Bind a host path to `/data` and pass your config.

### Index and query

```bash
docker run --rm -v "$(pwd)":/data ghcr.io/gaspardpetit/findx:latest index --config /data/findx.toml
docker run --rm -v "$(pwd)":/data ghcr.io/gaspardpetit/findx:latest query --config /data/findx.toml --mode keyword "project timeline"
```

### Watch and exec

```bash
docker run -d --name li -v "$(pwd)":/data ghcr.io/gaspardpetit/findx:latest watch --config /data/findx.toml
docker exec li findx query --config /data/findx.toml --mode keyword "project timeline"
```


## Help

```bash
cargo run -- --help
```
