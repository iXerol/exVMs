use std::path::{Path, PathBuf};

use crate::db::{self, Recording};
use crate::storage::{self, VoiceMemosStore};
use crate::{ExvmsError, Result};

pub trait StoreResolver {
    fn resolve_store(&self) -> Result<VoiceMemosStore>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultStoreResolver;

impl StoreResolver for DefaultStoreResolver {
    fn resolve_store(&self) -> Result<VoiceMemosStore> {
        storage::discover()
    }
}

#[derive(Debug, Clone)]
pub struct FixedStoreResolver {
    store: VoiceMemosStore,
}

impl FixedStoreResolver {
    pub fn new(recordings_dir: impl Into<PathBuf>, db_path: impl Into<PathBuf>) -> Self {
        Self {
            store: VoiceMemosStore {
                recordings_dir: recordings_dir.into(),
                db_path: db_path.into(),
            },
        }
    }

    pub fn from_store(store: VoiceMemosStore) -> Self {
        Self { store }
    }
}

impl StoreResolver for FixedStoreResolver {
    fn resolve_store(&self) -> Result<VoiceMemosStore> {
        Ok(self.store.clone())
    }
}

pub trait RecordingReader {
    fn load_recordings(&self, db_path: &Path) -> Result<Vec<Recording>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteRecordingReader;

impl RecordingReader for SqliteRecordingReader {
    fn load_recordings(&self, db_path: &Path) -> Result<Vec<Recording>> {
        db::load_recordings(db_path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingCatalog {
    pub store: VoiceMemosStore,
    pub recordings: Vec<Recording>,
}

#[derive(Debug, Clone)]
pub struct VoiceMemosCatalog<R = DefaultStoreResolver, D = SqliteRecordingReader> {
    store_resolver: R,
    recording_reader: D,
}

impl Default for VoiceMemosCatalog<DefaultStoreResolver, SqliteRecordingReader> {
    fn default() -> Self {
        Self {
            store_resolver: DefaultStoreResolver,
            recording_reader: SqliteRecordingReader,
        }
    }
}

impl<R, D> VoiceMemosCatalog<R, D>
where
    R: StoreResolver,
    D: RecordingReader,
{
    pub fn new(store_resolver: R, recording_reader: D) -> Self {
        Self {
            store_resolver,
            recording_reader,
        }
    }

    pub fn load(&self) -> Result<RecordingCatalog> {
        let store = self.store_resolver.resolve_store()?;
        let recordings = self.recording_reader.load_recordings(&store.db_path)?;
        Ok(RecordingCatalog { store, recordings })
    }
}

#[derive(Debug, Clone, Default)]
pub struct MockRecordingReader {
    recordings: Vec<Recording>,
}

impl MockRecordingReader {
    pub fn new(recordings: Vec<Recording>) -> Self {
        Self { recordings }
    }
}

impl RecordingReader for MockRecordingReader {
    fn load_recordings(&self, _db_path: &Path) -> Result<Vec<Recording>> {
        Ok(self.recordings.clone())
    }
}

#[derive(Debug, Clone)]
pub struct FailingStoreResolver {
    message: String,
}

impl FailingStoreResolver {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl StoreResolver for FailingStoreResolver {
    fn resolve_store(&self) -> Result<VoiceMemosStore> {
        Err(ExvmsError::StorageAccessDenied(PathBuf::from(
            self.message.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{RecordingPathKind, RecordingStoreKind};

    fn recording(id: i64) -> Recording {
        Recording {
            id,
            title: format!("Recording {id}"),
            recorded_at: id as f64,
            duration: 1.0,
            local_duration: 1.0,
            eviction_date: None,
            db_path: format!("{id}.m4a"),
            path_kind: RecordingPathKind::Relative,
            store_kind: RecordingStoreKind::CloudRecordings,
            folder_id: None,
            folder_name: None,
        }
    }

    #[test]
    fn catalog_loads_from_injected_store_and_reader() {
        let tmp = tempfile::tempdir().unwrap();
        let recordings_dir = tmp.path().join("Recordings");
        let db_path = recordings_dir.join("CloudRecordings.db");
        let resolver = FixedStoreResolver::new(&recordings_dir, &db_path);
        let reader = MockRecordingReader::new(vec![recording(7)]);
        let catalog = VoiceMemosCatalog::new(resolver, reader).load().unwrap();

        assert_eq!(catalog.recordings.len(), 1);
        assert_eq!(catalog.recordings[0].id, 7);
        assert_eq!(catalog.store.recordings_dir, recordings_dir);
        assert!(catalog.store.db_path.ends_with("CloudRecordings.db"));
    }

    #[test]
    fn fixed_store_resolver_can_point_to_recordings_db() {
        let tmp = tempfile::tempdir().unwrap();
        let recordings_dir = tmp.path().join("Recordings");
        let resolver =
            FixedStoreResolver::new(&recordings_dir, recordings_dir.join("Recordings.db"));
        let store = resolver.resolve_store().unwrap();

        assert_eq!(store.recordings_dir, recordings_dir);
        assert_eq!(store.db_path.file_name().unwrap(), "Recordings.db");
    }
}
