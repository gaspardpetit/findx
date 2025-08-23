# AGENTS

This repository contains `localindex`, a Rust CLI for indexing and searching local documents.

## Project Structure
- `src/` – Rust source code and module scaffolding
- `docs/` – project documentation
- `examples/` – example configurations or snippets
- `tools/` – helper scripts and container files
- `localindex.toml` – sample configuration
- Content extraction sidecar configured via `extractor_url` populates a `documents` table

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

## Documentation
Keep documentation current. Update `README.md` and this `AGENTS.md` whenever project behavior or structure changes.

## PR Checklist
- [ ] `cargo fmt --all`
- [ ] `cargo check`
- [ ] `cargo test`
- [ ] Docs updated (`README.md`, `AGENTS.md`, others)
