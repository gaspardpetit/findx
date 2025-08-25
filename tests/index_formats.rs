use camino::Utf8PathBuf;
use std::{fs, process::Command};
use tempfile::tempdir;

use findx::config::{Config, EmbeddingConfig};
use findx::{bus::EventBus, fs as findx_fs, index, metadata, search};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[test]
#[ignore]
fn indexes_various_document_types() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

    // Verify docling is available; skip test otherwise
    if Command::new("docling").arg("--version").output().is_err() {
        eprintln!("docling not installed; skipping integration test");
        return Ok(());
    }

    // Copy fixtures into temp root
    let fixtures = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for file in [
        "colors.docx",
        "pokemon_text.pdf",
        "pokemon_image.pdf",
        "plants.rtf",
        "fruits.md",
        "animals.txt",
    ] {
        fs::copy(fixtures.join(file), root.join(file))?;
    }

    let extractor = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/docling_stdout.sh");

    // Ensure docling can process PDFs (requires model downloads). Skip if conversion fails.
    if Command::new(extractor.as_str())
        .arg(fixtures.join("pokemon_text.pdf"))
        .output()
        .map_or(true, |o| !o.status.success())
    {
        eprintln!("docling could not process PDFs; skipping integration test");
        return Ok(());
    }

    let cfg = Config {
        db: root.join("catalog.db"),
        tantivy_index: root.join("idx"),
        roots: vec![root.clone()],
        include: vec!["**/*".into()],
        exclude: vec![],
        max_file_size_mb: 200,
        follow_symlinks: false,
        include_hidden: false,
        allow_offline_hydration: false,
        commit_interval_secs: 45,
        guard_interval_secs: 180,
        default_language: "en".into(),
        extractor_cmd: extractor.as_str().into(),
        embedding: EmbeddingConfig {
            provider: "disabled".into(),
        },
        mirror: findx::config::MirrorConfig {
            root: root.join("raw"),
        },
        bus: findx::config::BusConfig {
            bounds: findx::config::BusBounds {
                source_fs: 16,
                mirror_text: 16,
            },
        },
        extract: findx::config::ExtractConfig {
            pool_size: 1,
            jobs_bound: 16,
        },
    };

    // Scan filesystem and extract contents (legacy path pending new pipeline)
    let conn = findx::db::open(&cfg.db)?;
    let bus = EventBus::new(&cfg.bus.bounds, Arc::new(Mutex::new(conn)));
    let bus_meta = bus.clone();
    let cfg_meta = cfg.clone();
    let stop_meta = Arc::new(AtomicBool::new(false));
    let stop_thread = stop_meta.clone();
    let handle = std::thread::spawn(move || {
        let _ = metadata::run(bus_meta, &cfg_meta, &stop_thread);
    });
    let mut state = findx_fs::FsState::default();
    findx_fs::cold_scan(&cfg, &bus, &mut state)?;
    // Build indexes
    index::reindex_all(&cfg, None)?;

    let cases = [
        ("red", "colors.docx"),
        ("Pikachu", "pokemon_text.pdf"),
        ("Bulbasaur", "pokemon_image.pdf"),
        ("fern", "plants.rtf"),
        ("banana", "fruits.md"),
        ("otter", "animals.txt"),
    ];

    for (query, filename) in cases {
        let res = search::keyword(&cfg, query, 10)?;
        assert!(
            res.results.iter().any(|h| h.path.ends_with(filename)),
            "query '{query}' did not return '{filename}'"
        );
    }

    stop_meta.store(true, Ordering::SeqCst);
    handle.join().unwrap();
    Ok(())
}
