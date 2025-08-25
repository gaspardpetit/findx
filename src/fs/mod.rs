use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::bus::EventBus;
use crate::config::Config;
use crate::events::{FileMeta, FileMove, SourceEvent};
use anyhow::Result;
use camino::{Utf8Path, Utf8PathBuf};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// In-memory state of previously seen files keyed by `file_uid`.
#[derive(Default)]
pub struct FsState {
    files: HashMap<String, FileInfo>,
}

#[derive(Clone)]
struct FileInfo {
    file_uid: String,
    path: Utf8PathBuf,
    size: u64,
    mtime_ns: i64,
    fast_sig: String,
    is_offline: bool,
    attrs: u64,
}

/// Perform a full scan over configured roots and publish a `SyncDelta` event with
/// additions, modifications, moves, and deletions compared to the previous state.
pub fn cold_scan(cfg: &Config, bus: &EventBus, state: &mut FsState) -> Result<()> {
    let include = build_glob_set(&cfg.include)?;
    let exclude = build_glob_set(&cfg.exclude)?;
    let mut current: HashMap<String, FileInfo> = HashMap::new();

    for root in &cfg.roots {
        if !root.exists() {
            anyhow::bail!("root path not found: {}", root);
        }
        let walker = WalkBuilder::new(root)
            .hidden(false)
            .follow_links(cfg.follow_symlinks)
            .build();
        for dent in walker {
            let dent = match dent {
                Ok(d) => d,
                Err(_) => continue,
            };
            if !dent.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }
            let path = match Utf8Path::from_path(dent.path()) {
                Some(p) => p.to_owned(),
                None => continue,
            };
            if !cfg.include_hidden {
                if path
                    .file_name()
                    .map(|n| n.starts_with('.'))
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            let mirror_root = if cfg.mirror.root.is_absolute() {
                cfg.mirror.root.clone()
            } else {
                root.join(&cfg.mirror.root)
            };
            if path.starts_with(&mirror_root) {
                continue;
            }
            if !include.is_match(path.as_std_path()) || exclude.is_match(path.as_std_path()) {
                continue;
            }
            let info = gather_info(&path)?;
            current.insert(info.file_uid.clone(), info);
        }
    }

    emit_delta(bus, state, &current)?;
    *state = FsState { files: current };
    Ok(())
}

/// Watch for filesystem changes and periodically rescan roots. Multiple rapid
/// changes are coalesced into a single `SyncDelta` event via a 300ms debounce.
pub fn watch(cfg: &Config, bus: EventBus, stop: &AtomicBool) -> Result<()> {
    let mut state = FsState::default();
    cold_scan(cfg, &bus, &mut state)?;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )?;
    for root in &cfg.roots {
        watcher.watch(root.as_std_path(), RecursiveMode::Recursive)?;
    }

    let debounce = Duration::from_millis(300);
    let mut last_event: Option<Instant> = None;

    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(_event)) => {
                last_event = Some(Instant::now());
            }
            Ok(Err(_)) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if let Some(t) = last_event {
            if t.elapsed() > debounce {
                cold_scan(cfg, &bus, &mut state)?;
                last_event = None;
            }
        }
    }
    Ok(())
}

fn emit_delta(bus: &EventBus, state: &FsState, current: &HashMap<String, FileInfo>) -> Result<()> {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut moved = Vec::new();

    for info in current.values() {
        if let Some(old) = state.files.get(&info.file_uid) {
            if old.path != info.path {
                moved.push(FileMove {
                    file_uid: info.file_uid.clone(),
                    from: old.path.clone(),
                    to: info.path.clone(),
                });
            } else if old.fast_sig != info.fast_sig {
                modified.push(to_meta(info));
            }
        } else {
            added.push(to_meta(info));
        }
    }

    let deleted = state
        .files
        .iter()
        .filter(|(uid, _)| !current.contains_key(*uid))
        .map(|(_, info)| to_meta(info))
        .collect::<Vec<_>>();

    if added.is_empty() && modified.is_empty() && moved.is_empty() && deleted.is_empty() {
        return Ok(());
    }

    bus.publish_source(SourceEvent::SyncDelta {
        added,
        modified,
        moved,
        deleted,
    })?;
    Ok(())
}

fn to_meta(info: &FileInfo) -> FileMeta {
    FileMeta {
        file_uid: info.file_uid.clone(),
        path: info.path.clone(),
        size: info.size,
        mtime_ns: info.mtime_ns,
        fast_sig: info.fast_sig.clone(),
        is_offline: info.is_offline,
        attrs: info.attrs,
    }
}

fn gather_info(path: &Utf8Path) -> Result<FileInfo> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let mtime_ns = meta.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() as i64;
    let (fast_sig, is_offline, attrs) = compute_fast_sig(&meta);
    let file_uid = compute_file_uid(&meta, path);
    Ok(FileInfo {
        file_uid,
        path: path.to_owned(),
        size,
        mtime_ns,
        fast_sig,
        is_offline,
        attrs,
    })
}

fn compute_file_uid(meta: &std::fs::Metadata, _path: &Utf8Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!("ux-{}:{}", meta.dev(), meta.ino())
    }
    #[cfg(not(unix))]
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        let _ = _path;
        hasher.update(meta.len().to_le_bytes());
        format!("fp-{:x}", hasher.finalize())
    }
}

fn build_glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p)?);
    }
    Ok(builder.build()?)
}

fn compute_fast_sig(meta: &std::fs::Metadata) -> (String, bool, u64) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_OFFLINE: u32 = 0x00001000;
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x00400000;
        let attrs = meta.file_attributes();
        let is_offline =
            attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0;
        let fast_sig = format!(
            "{:x}:{:x}:{:x}:{:x}:{:x}",
            meta.file_index_high(),
            meta.file_index_low(),
            meta.file_size(),
            meta.last_write_time(),
            attrs
        );
        (fast_sig, is_offline, attrs as u64)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let fast_sig = format!(
            "{:x}:{:x}:{:x}:{:x}:{:x}",
            meta.dev(),
            meta.ino(),
            meta.size(),
            meta.mtime_nsec(),
            meta.ctime_nsec()
        );
        (fast_sig, false, 0)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let fast_sig = format!("{}:{}", meta.len(), mtime);
        (fast_sig, false, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::{
        bus::EventBus,
        config::{BusBounds, BusConfig, ExtractConfig, MirrorConfig},
    };
    use std::sync::{atomic::AtomicBool, Arc, Mutex};
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    #[ignore]
    fn debounced_events_single_syncdelta() -> Result<()> {
        let tmp = tempdir()?;
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        std::fs::write(root.join("a.txt"), b"hello")?;

        let cfg = crate::config::Config {
            db: root.join("catalog.db"),
            tantivy_index: Utf8PathBuf::from("idx"),
            roots: vec![root.clone()],
            include: vec!["**/*.txt".into()],
            exclude: vec![],
            max_file_size_mb: 200,
            follow_symlinks: false,
            include_hidden: false,
            allow_offline_hydration: false,
            commit_interval_secs: 45,
            guard_interval_secs: 180,
            default_language: "auto".into(),
            extractor_cmd: String::new(),
            embedding: crate::config::EmbeddingConfig {
                provider: "disabled".into(),
            },
            mirror: MirrorConfig {
                root: Utf8PathBuf::from("raw"),
            },
            bus: BusConfig {
                bounds: BusBounds {
                    source_fs: 16,
                    mirror_text: 16,
                },
            },
            extract: ExtractConfig {
                pool_size: 1,
                jobs_bound: 16,
            },
        };

        let conn = db::open(&cfg.db)?;
        let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
        let rx = bus.subscribe_source();
        let stop = Arc::new(AtomicBool::new(false));
        let bus_watcher = bus.clone();
        let cfg_watcher = cfg.clone();
        let stop_watcher = stop.clone();
        let handle = std::thread::spawn(move || {
            watch(&cfg_watcher, bus_watcher, &stop_watcher).unwrap();
        });

        // Consume initial added event
        let _initial = rx.recv().unwrap();

        // Burst of modifications
        for _ in 0..3 {
            std::fs::write(root.join("a.txt"), b"world")?;
        }
        std::thread::sleep(Duration::from_millis(500));

        // Expect only one SyncDelta for modifications
        let env = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        match env.data {
            SourceEvent::SyncDelta { modified, .. } => {
                assert_eq!(modified.len(), 1);
            }
            _ => panic!("unexpected event"),
        }

        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        Ok(())
    }
}
