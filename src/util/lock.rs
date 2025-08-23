use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process;

use camino::Utf8PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LockError {
    #[error("lockfile exists at {0}")]
    Exists(Utf8PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Lockfile {
    path: Utf8PathBuf,
}

impl Lockfile {
    pub fn acquire(path: Utf8PathBuf) -> Result<Self, LockError> {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.as_std_path())
        {
            Ok(mut f) => {
                writeln!(f, "{}", process::id())?;
                Ok(Self { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(LockError::Exists(path)),
            Err(e) => Err(LockError::Io(e)),
        }
    }
}

impl Drop for Lockfile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
