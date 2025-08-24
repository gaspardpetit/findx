use std::sync::Arc;
use std::time::Duration;

use atty::Stream;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::OnceCell;

/// Textual dashboard for indexing progress.
#[derive(Clone)]
pub struct Dashboard {
    mp: Arc<MultiProgress>,
    files: ProgressBar,
    chunks: ProgressBar,
}

impl Dashboard {
    /// Create a dashboard if STDOUT is a TTY. Returns `None` when
    /// running in a non-console context.
    pub fn new(total_files: u64) -> Option<Self> {
        if !atty::is(Stream::Stdout) {
            return None;
        }
        let mp = MultiProgress::new();
        let file_style =
            ProgressStyle::with_template("{prefix:<7} {msg:<40} {wide_bar} {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("##-");
        let files = mp.add(ProgressBar::new(total_files));
        files.set_style(file_style);
        files.set_prefix("Files");
        let chunk_style =
            ProgressStyle::with_template("{prefix:<7} {wide_bar} {pos}/{len} ({eta})")
                .unwrap()
                .progress_chars("##-");
        let chunks = mp.add(ProgressBar::new(0));
        chunks.set_style(chunk_style);
        chunks.set_prefix("Chunks");
        Some(Self {
            mp: Arc::new(mp),
            files,
            chunks,
        })
    }

    /// Increment the file progress bar.
    pub fn inc_file(&self) {
        self.files.inc(1);
    }

    /// Update the file progress bar message with the current path.
    pub fn set_file(&self, path: &str) {
        self.files.set_message(path.to_string());
    }

    /// Mark file progress bar as finished.
    pub fn finish_files(&self) {
        self.files.finish();
    }

    /// Set the total number of chunks once known.
    pub fn set_chunk_len(&self, len: u64) {
        self.chunks.set_length(len);
    }

    /// Increment the chunk progress bar.
    pub fn inc_chunk(&self) {
        self.chunks.inc(1);
    }

    /// Mark chunk progress bar as finished.
    pub fn finish_chunks(&self) {
        self.chunks.finish();
    }

    /// Add a persistent spinner used in watch mode.
    pub fn watch_spinner(&self) -> ProgressBar {
        let spinner = self.mp.add(ProgressBar::new_spinner());
        spinner.set_message("Watching for changes");
        spinner.enable_steady_tick(Duration::from_millis(100));
        spinner
    }
}

/// Global dashboard used by long running commands.
static DASHBOARD: OnceCell<Dashboard> = OnceCell::new();

/// Initialize and store a global dashboard.
pub fn init(total_files: u64) {
    if DASHBOARD.get().is_none() {
        if let Some(d) = Dashboard::new(total_files) {
            let _ = DASHBOARD.set(d);
        }
    }
}

/// Get a reference to the global dashboard, if any.
pub fn get() -> Option<&'static Dashboard> {
    DASHBOARD.get()
}
