use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process;

use camino::Utf8PathBuf;
use thiserror::Error;
use tracing::debug;

#[derive(Debug, Error)]
pub enum LockError {
    #[error("lockfile exists at {0}")]
    Exists(Utf8PathBuf),
    #[error("could not create lockfile at {path}: {source}")]
    Io {
        path: Utf8PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug)]
pub struct Lockfile {
    path: Utf8PathBuf,
}

impl Lockfile {
    pub fn acquire(path: Utf8PathBuf) -> Result<Self, LockError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| LockError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        debug!(%path, "acquiring lockfile");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_std_path())
        {
            Ok(mut f) => {
                writeln!(f, "{}", process::id()).map_err(|e| LockError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                Ok(Self { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(LockError::Exists(path)),
            Err(e) => Err(LockError::Io { path, source: e }),
        }
    }
}

impl Drop for Lockfile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
