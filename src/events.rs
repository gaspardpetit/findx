use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMeta {
    pub file_uid: String,
    pub path: Utf8PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMove {
    pub file_uid: String,
    pub from: Utf8PathBuf,
    pub to: Utf8PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceEvent {
    SyncStarted,
    SyncDelta {
        added: Vec<FileMeta>,
        modified: Vec<FileMeta>,
        moved: Vec<FileMove>,
        deleted: Vec<FileMeta>,
    },
    FileAdded {
        file_uid: String,
        path: Utf8PathBuf,
    },
    FileModified {
        file_uid: String,
        path: Utf8PathBuf,
    },
    FileMoved {
        file_uid: String,
        from: Utf8PathBuf,
        to: Utf8PathBuf,
    },
    FileDeleted {
        file_uid: String,
        path: Utf8PathBuf,
    },
    ExtractionRequested {
        file_uid: String,
        content_hash: String,
    },
    ExtractionCompleted {
        file_uid: String,
        content_hash: String,
    },
    ExtractionFailed {
        file_uid: String,
        error: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MirrorEvent {
    MirrorDocUpserted {
        file_uid: String,
        content_hash: String,
    },
    MirrorDocDeleted {
        file_uid: String,
    },
    MirrorChunkUpserted {
        chunk_id: String,
        file_uid: String,
        order: u64,
    },
    MirrorChunkDeleted {
        chunk_id: String,
        file_uid: String,
    },
}
