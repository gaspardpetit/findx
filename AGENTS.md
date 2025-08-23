# AGENTS

This repository contains `localindex`, a Rust CLI for indexing and searching local documents.

## Project Structure
- `src/` – Rust source code and module scaffolding
- `docs/` – project documentation
- `doc/` – dependency listings
- `examples/` – example configurations or snippets
- `.github/workflows/` – CI and release automation
- `tools/` – helper scripts and container files
- `Dockerfile` – container image for running the CLI
- `localindex.toml` – sample configuration
- Content extraction uses a configurable command (`extractor_cmd`, default `docling --to txt`) to populate a `documents` table; plain text files are read directly
- Tantivy-based BM25 index built under `tantivy_index`
- Chunk index stored under `tantivy_index/chunks`
- Embeddings stored in SQLite `embeddings` table for semantic search.
  Local embeddings use the `fastembed` crate by default; set `EMBEDDING_URL`
  (and optional `EMBEDDING_API_KEY`) to delegate to an external provider.
- `watch` listens for SIGINT and SIGTERM to exit cleanly.

## Standards
- Rust 1.75+
- Format code with `cargo fmt --all`
- Prefer `Utf8PathBuf` for paths and `tracing` for logs

## Build and Test
To accept a change, run:

```bash
cargo fmt --all
cargo check
cargo test
```

Snapshot artifacts for `main` come from `.github/workflows/snapshot.yml`.
Releases are published with `.github/workflows/release.yml`, which builds and uploads binaries for Linux, macOS, and Windows when a tag is pushed.

## Documentation
Keep documentation current. Update `README.md` and this `AGENTS.md` whenever project behavior or structure changes.

## PR Checklist
- [ ] `cargo fmt --all`
- [ ] `cargo check`
- [ ] `cargo test`
- [ ] Docs updated (`README.md`, `AGENTS.md`, others)
