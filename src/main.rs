mod bus;
mod chunk;
mod cli;
mod config;
mod db;
mod embed;
mod events;
mod extract;
mod fs;
mod index;
mod metadata;
mod mirror;
mod search;
mod util;

use anyhow::Result;
use camino::Utf8PathBuf;
use clap::Parser;
use cli::{Cli, Command, OneshotArgs, WatchArgs};
use serde::Serialize;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use util::logging;
use util::{dashboard, lock::Lockfile};

fn print_json<T: Serialize>(res: &T, compact: bool) -> Result<()> {
    let json = if compact {
        serde_json::to_string(res)?
    } else {
        serde_json::to_string_pretty(res)?
    };
    println!("{}", json);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.log_format);

    let mut cfg = match config::Config::load(&cli.config) {
        Ok(c) => c,
        Err(_) => config::Config::default(),
    };

    let conn = db::open(&cfg.db)?;
    let bus = bus::EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
    let bus_meta = bus.clone();
    let cfg_meta = cfg.clone();
    let meta_stop = Arc::new(AtomicBool::new(false));
    let meta_stop_thread = meta_stop.clone();
    std::thread::spawn(move || {
        let _ = metadata::run(bus_meta, &cfg_meta, &meta_stop_thread);
    });
    let bus_extract = bus.clone();
    let cfg_extract = cfg.clone();
    let extract_stop = Arc::new(AtomicBool::new(false));
    let extract_stop_thread = extract_stop.clone();
    std::thread::spawn(move || {
        let _ = extract::run_pool(bus_extract, &cfg_extract, &extract_stop_thread);
    });
    let bus_mirror = bus.clone();
    let cfg_mirror = cfg.clone();
    let mirror_stop = Arc::new(AtomicBool::new(false));
    let mirror_stop_thread = mirror_stop.clone();
    std::thread::spawn(move || {
        let _ = mirror::run(bus_mirror, &cfg_mirror, &mirror_stop_thread);
    });
    let mut fs_state = fs::FsState::default();

    match &cli.command {
        Command::Index(args)
        | Command::Watch(WatchArgs { index: args, .. })
        | Command::Oneshot(OneshotArgs { index: args, .. }) => {
            if !args.roots.is_empty() {
                cfg.roots = args.roots.clone();
            }
            if let Some(db) = &args.db {
                cfg.db = db.clone();
            }
            if let Some(idx) = &args.tantivy_index {
                cfg.tantivy_index = idx.clone();
            }
            if let Some(cmd) = &args.extractor_cmd {
                cfg.extractor_cmd = cmd.clone();
            }
        }
        _ => {}
    }
    match &cli.command {
        Command::Query(args) | Command::Oneshot(OneshotArgs { query: args, .. }) => {
            if let Some(db) = &args.db {
                cfg.db = db.clone();
            }
            if let Some(idx) = &args.tantivy_index {
                cfg.tantivy_index = idx.clone();
            }
        }
        _ => {}
    }

    let _lock = match &cli.command {
        Command::Index(_) | Command::Watch(_) | Command::Oneshot(_) => {
            let lock_path = Utf8PathBuf::from(".findx/state/index.lock");
            Some(Lockfile::acquire(lock_path)?)
        }
        _ => None,
    };

    match &cli.command {
        Command::Index(_) => {
            tracing::info!(?cfg, "index");
            fs::cold_scan(&cfg, &bus, &mut fs_state)?;
            let conn = db::open(&cfg.db)?;
            let total_files: i64 = conn.query_row(
                "SELECT COUNT(*) FROM files WHERE status='active'",
                [],
                |r| r.get(0),
            )?;
            dashboard::init(total_files as u64);
            let dash = dashboard::get();
            index::reindex_all_with_retry(&cfg, dash, 3)?;
        }
        Command::Watch(w) => {
            tracing::info!(threads = w.threads, ?cfg, "watch");
            let stop = AtomicBool::new(false);
            fs::watch(&cfg, bus.clone(), &stop)?;
        }
        Command::Query(q) => {
            if !cfg.db.exists() || !cfg.tantivy_index.exists() {
                println!("No index found, creating one under {:?}", cfg.tantivy_index);
                fs::cold_scan(&cfg, &bus, &mut fs_state)?;
                let conn = db::open(&cfg.db)?;
                let total_files: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM files WHERE status='active'",
                    [],
                    |r| r.get(0),
                )?;
                dashboard::init(total_files as u64);
                let dash = dashboard::get();
                index::reindex_all_with_retry(&cfg, dash, 3)?;
            }
            tracing::info!(mode = ?q.mode, query = %q.query, top_k = q.top_k, chunks = q.chunks, ?cfg, "query");
            match q.mode {
                cli::QueryMode::Keyword => {
                    if q.chunks {
                        let res = search::keyword_chunks(&cfg, &q.query, q.top_k)?;
                        print_json(&res, cli.compact_output)?;
                    } else {
                        let res = search::keyword(&cfg, &q.query, q.top_k)?;
                        print_json(&res, cli.compact_output)?;
                    }
                }
                cli::QueryMode::Semantic => {
                    let res = search::semantic_chunks(&cfg, &q.query, q.top_k)?;
                    print_json(&res, cli.compact_output)?;
                }
                cli::QueryMode::Hybrid => {
                    let res = search::hybrid_chunks(&cfg, &q.query, q.top_k)?;
                    print_json(&res, cli.compact_output)?;
                }
            }
        }
        Command::Oneshot(o) => {
            tracing::info!(mode = ?o.query.mode, query = %o.query.query, ?cfg, "oneshot");
            fs::cold_scan(&cfg, &bus, &mut fs_state)?;
            let conn = db::open(&cfg.db)?;
            let total_files: i64 = conn.query_row(
                "SELECT COUNT(*) FROM files WHERE status='active'",
                [],
                |r| r.get(0),
            )?;
            dashboard::init(total_files as u64);
            let dash = dashboard::get();
            index::reindex_all_with_retry(&cfg, dash, 3)?;
            match o.query.mode {
                cli::QueryMode::Keyword => {
                    if o.query.chunks {
                        let res = search::keyword_chunks(&cfg, &o.query.query, o.query.top_k)?;
                        print_json(&res, cli.compact_output)?;
                    } else {
                        let res = search::keyword(&cfg, &o.query.query, o.query.top_k)?;
                        print_json(&res, cli.compact_output)?;
                    }
                }
                cli::QueryMode::Semantic => {
                    let res = search::semantic_chunks(&cfg, &o.query.query, o.query.top_k)?;
                    print_json(&res, cli.compact_output)?;
                }
                cli::QueryMode::Hybrid => {
                    let res = search::hybrid_chunks(&cfg, &o.query.query, o.query.top_k)?;
                    print_json(&res, cli.compact_output)?;
                }
            }
        }
        Command::Serve(s) => {
            tracing::info!(bind = %s.bind, "serve");
            println!("'serve' command is not implemented yet");
        }
        Command::Migrate(m) => {
            tracing::info!(check = m.check, apply = m.apply, "migrate");
            println!("'migrate' command is not implemented yet");
        }
        Command::Status => {
            tracing::info!("status");
            println!("'status' command is not implemented yet");
        }
    }

    Ok(())
}
