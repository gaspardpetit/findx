[![Rust](https://github.com/gaspardpetit/localindex/actions/workflows/rust.yml/badge.svg)](https://github.com/gaspardpetit/localindex/actions/workflows/rust.yml)
# localindex

`localindex` is a Rust CLI for indexing and searching local documents. This repository currently contains the foundational scaffolding for configuration, logging, locking, and the command-line interface.

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

[embedding]
provider = "disabled"
```

## Filesystem cataloging

The `index` command performs a cold scan of the configured roots and
stores file metadata in a SQLite database (`files` and `ops_log` tables).
The `watch` command runs the scan and then watches for filesystem
changes, updating the catalog as files are added, modified, or deleted.

## Building

```bash
cargo build
```

## Help

```bash
cargo run -- --help
```
