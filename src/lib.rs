pub mod db;
pub mod export;
pub mod macos;
pub mod sanitize;
pub mod storage;
pub mod time;

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ExvmsError>;

#[derive(Debug, Error)]
pub enum ExvmsError {
    #[error("Voice Memos recordings directory was not found")]
    StorageNotFound,
    #[error("Voice Memos database was not found in {0}")]
    DatabaseNotFound(PathBuf),
    #[error("missing required column(s) in {table}: {columns:?}")]
    MissingColumns {
        table: &'static str,
        columns: Vec<&'static str>,
    },
    #[error("recording id(s) not found: {0:?}")]
    MissingIds(Vec<i64>),
    #[error(
        "recording id(s) are cloud-only or incomplete and cannot be exported: {0:?}. Open/download them in Voice Memos.app first"
    )]
    CloudOnlyRequested(Vec<i64>),
    #[error("no local-exportable recordings were selected")]
    NoExportableRecordings,
    #[error("unsafe source path for recording {id}: {path}")]
    UnsafeSourcePath { id: i64, path: String },
    #[error("target already exists for recording {id}: {path}")]
    TargetExists { id: i64, path: PathBuf },
    #[error("multiple recordings would export to the same target: {path}: ids {ids:?}")]
    TargetCollision { path: PathBuf, ids: Vec<i64> },
    #[error("failed to classify local availability for recording {id}: {message}")]
    Availability { id: i64, message: String },
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("SQLite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("macOS API error: {0}")]
    Macos(String),
}
