mod chunk;
mod cli;
mod config;
mod db;
mod embed;
mod extract;
mod fs;
mod index;
mod search;
mod util;

use anyhow::Result;
use camino::Utf8PathBuf;
use clap::Parser;
use cli::{Cli, Command, OneshotArgs, WatchArgs};
use util::lock::Lockfile;
use util::logging;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.log_format);

    let mut cfg = match config::Config::load(&cli.config) {
        Ok(c) => c,
        Err(_) => config::Config::default(),
    };

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
            let lock_path = Utf8PathBuf::from("state/index.lock");
            Some(Lockfile::acquire(lock_path)?)
        }
        _ => None,
    };

    match &cli.command {
        Command::Index(_) => {
            tracing::info!(?cfg, "index");
            fs::cold_scan(&cfg)?;
            index::reindex_all(&cfg)?;
        }
        Command::Watch(w) => {
            tracing::info!(threads = w.threads, ?cfg, "watch");
            fs::watch(&cfg)?;
        }
        Command::Query(q) => {
            tracing::info!(mode = ?q.mode, query = %q.query, top_k = q.top_k, chunks = q.chunks, ?cfg, "query");
            match q.mode {
                cli::QueryMode::Keyword => {
                    if q.chunks {
                        let res = search::keyword_chunks(&cfg, &q.query, q.top_k)?;
                        println!("{}", serde_json::to_string(&res)?);
                    } else {
                        let res = search::keyword(&cfg, &q.query, q.top_k)?;
                        println!("{}", serde_json::to_string(&res)?);
                    }
                }
                cli::QueryMode::Semantic => {
                    let res = search::semantic_chunks(&cfg, &q.query, q.top_k)?;
                    println!("{}", serde_json::to_string(&res)?);
                }
                cli::QueryMode::Hybrid => {
                    let res = search::hybrid_chunks(&cfg, &q.query, q.top_k)?;
                    println!("{}", serde_json::to_string(&res)?);
                }
            }
        }
        Command::Oneshot(o) => {
            tracing::info!(mode = ?o.query.mode, query = %o.query.query, ?cfg, "oneshot");
            fs::cold_scan(&cfg)?;
            index::reindex_all(&cfg)?;
            match o.query.mode {
                cli::QueryMode::Keyword => {
                    if o.query.chunks {
                        let res = search::keyword_chunks(&cfg, &o.query.query, o.query.top_k)?;
                        println!("{}", serde_json::to_string(&res)?);
                    } else {
                        let res = search::keyword(&cfg, &o.query.query, o.query.top_k)?;
                        println!("{}", serde_json::to_string(&res)?);
                    }
                }
                cli::QueryMode::Semantic => {
                    let res = search::semantic_chunks(&cfg, &o.query.query, o.query.top_k)?;
                    println!("{}", serde_json::to_string(&res)?);
                }
                cli::QueryMode::Hybrid => {
                    let res = search::hybrid_chunks(&cfg, &o.query.query, o.query.top_k)?;
                    println!("{}", serde_json::to_string(&res)?);
                }
            }
        }
        Command::Serve(s) => {
            tracing::info!(bind = %s.bind, "serve");
        }
        Command::Migrate(m) => {
            tracing::info!(check = m.check, apply = m.apply, "migrate");
        }
        Command::Status => {
            tracing::info!("status");
        }
    }

    Ok(())
}
