# Dependencies

The project aims to work out of the box with minimal external requirements. The tables below list build-time and run-time dependencies, whether they are mandatory, and their roles.

## Build dependencies

| Dependency | Mandatory? | Purpose |
| --- | --- | --- |
| Rust 1.75+ | Yes | Required to build and run `localindex` from source |
| anyhow | Yes | Simplified error handling |
| camino (serde1) | Yes | UTF-8 path handling and serialization |
| clap (derive) | Yes | Command-line argument parsing |
| serde (derive) | Yes | Serialization/deserialization for configuration and data |
| thiserror | Yes | Derive macros for error types |
| tokio (rt-multi-thread, macros) | Yes | Asynchronous runtime |
| toml | Yes | Parse `localindex.toml` configuration files |
| tracing | Yes | Structured logging |
| tracing-subscriber (fmt, json, env-filter) | Yes | Logging subscriber for tracing |
| metrics | Yes | Application metrics collection |
| tantivy | Yes | Full-text search indexing engine |
| chrono (serde) | Yes | Date and time handling |
| num_cpus | Yes | Detect available CPU cores |
| rusqlite (bundled) | Yes | SQLite database with bundled library |
| walkdir | Yes | Recursive directory traversal |
| ignore | Yes | File pattern filtering |
| globset | Yes | Glob pattern matching |
| notify | Yes | Filesystem notifications |
| xxhash-rust (xxh3) | Yes | Hashing for file digests |
| blake3 | Yes | Cryptographic hashing for file content |
| serde_json | Yes | JSON serialization |
| tempfile (dev) | No | Used in tests for temporary files |

## Runtime dependencies

| Dependency | Mandatory? | Purpose |
| --- | --- | --- |
| docling CLI | No | Default document extractor for non-text formats; plain text and Markdown are handled internally |
| Embedding endpoint | No | Remote service for generating vector embeddings when semantic search is enabled |
| Bundled SQLite (via rusqlite) | Yes | On-disk database for metadata and embeddings; no external DB service required |

The SQLite library is statically linked via the `rusqlite` crate, so the application manages its database files without requiring a separate database server.

