use std::path::{Path, PathBuf};

use crate::{ExvmsError, Result, db};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceMemosStore {
    pub recordings_dir: PathBuf,
    pub db_path: PathBuf,
}

pub fn discover() -> Result<VoiceMemosStore> {
    let home = dirs::home_dir().ok_or(ExvmsError::StorageNotFound)?;
    discover_from_home(home)
}

pub fn discover_from_home(home: PathBuf) -> Result<VoiceMemosStore> {
    let mut first_existing_recordings_dir = None;
    let mut first_existing_database = None;
    let mut first_supported_empty_store = None;
    let mut first_probe_error = None;

    for dir in candidate_recording_dirs(&home) {
        if !dir.is_dir() {
            continue;
        }

        first_existing_recordings_dir.get_or_insert_with(|| dir.clone());

        for db_path in candidate_database_paths(&dir) {
            if !db_path.is_file() {
                continue;
            }

            first_existing_database.get_or_insert_with(|| db_path.clone());

            match db::probe_database(&db_path) {
                Ok(Some(probe)) if probe.row_count > 0 => {
                    return Ok(VoiceMemosStore {
                        recordings_dir: dir,
                        db_path,
                    });
                }
                Ok(Some(_)) => {
                    first_supported_empty_store.get_or_insert_with(|| VoiceMemosStore {
                        recordings_dir: dir.clone(),
                        db_path,
                    });
                }
                Ok(None) => {}
                Err(err) => {
                    first_probe_error.get_or_insert(err);
                }
            }
        }
    }

    if let Some(store) = first_supported_empty_store {
        return Ok(store);
    }

    if let Some(err) = first_probe_error {
        return Err(err);
    }

    if let Some(db_path) = first_existing_database {
        return Err(ExvmsError::UnsupportedDatabase(format!(
            "no supported recording table was found in {}",
            db_path.display()
        )));
    }

    if let Some(dir) = first_existing_recordings_dir {
        Err(ExvmsError::DatabaseNotFound(dir))
    } else {
        Err(ExvmsError::StorageNotFound)
    }
}

pub fn candidate_recording_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings"),
        home.join("Library/Containers/Voice Memos/Data/Library/Application Support/Recordings"),
        home.join("Library/Application Support/com.apple.voicememos/Recordings"),
    ]
}

pub fn candidate_database_paths(recordings_dir: &Path) -> Vec<PathBuf> {
    vec![
        recordings_dir.join("CloudRecordings.db"),
        recordings_dir.join("Recordings.db"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn candidate_dirs_are_in_expected_order() {
        let home = PathBuf::from("/Users/example");
        let dirs = candidate_recording_dirs(&home);
        assert_eq!(
            dirs[0],
            PathBuf::from(
                "/Users/example/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings"
            )
        );
        assert_eq!(
            dirs[1],
            PathBuf::from(
                "/Users/example/Library/Containers/Voice Memos/Data/Library/Application Support/Recordings"
            )
        );
        assert_eq!(
            dirs[2],
            PathBuf::from(
                "/Users/example/Library/Application Support/com.apple.voicememos/Recordings"
            )
        );
    }

    #[test]
    fn candidate_database_paths_try_cloud_then_recordings() {
        let dir = PathBuf::from("/tmp/Recordings");
        assert_eq!(
            candidate_database_paths(&dir),
            vec![dir.join("CloudRecordings.db"), dir.join("Recordings.db")]
        );
    }

    #[test]
    fn discovery_skips_existing_dir_without_database() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let modern =
            home.join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings");
        let legacy = home.join("Library/Application Support/com.apple.voicememos/Recordings");
        std::fs::create_dir_all(&modern).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();
        create_legacy_recordings_db(&legacy.join("Recordings.db"), true);

        let store = discover_from_home(home.to_path_buf()).unwrap();
        assert_eq!(store.recordings_dir, legacy);
        assert_eq!(store.db_path.file_name().unwrap(), "Recordings.db");
    }

    #[test]
    fn discovery_uses_recordings_db_when_cloud_db_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp
            .path()
            .join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings");
        std::fs::create_dir_all(&dir).unwrap();
        create_empty_cloud_db(&dir.join("CloudRecordings.db"));
        create_legacy_recordings_db(&dir.join("Recordings.db"), true);

        let store = discover_from_home(tmp.path().to_path_buf()).unwrap();
        assert_eq!(store.db_path.file_name().unwrap(), "Recordings.db");
    }

    #[test]
    fn discovery_allows_empty_supported_database() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp
            .path()
            .join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings");
        std::fs::create_dir_all(&dir).unwrap();
        create_empty_cloud_db(&dir.join("CloudRecordings.db"));

        let store = discover_from_home(tmp.path().to_path_buf()).unwrap();
        assert_eq!(store.db_path.file_name().unwrap(), "CloudRecordings.db");
    }

    #[test]
    fn discovery_reports_unsupported_database_when_db_exists_without_supported_tables() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp
            .path()
            .join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings");
        std::fs::create_dir_all(&dir).unwrap();
        Connection::open(dir.join("CloudRecordings.db"))
            .unwrap()
            .execute_batch("CREATE TABLE ZUNRELATED (Z_PK INTEGER PRIMARY KEY);")
            .unwrap();

        let err = discover_from_home(tmp.path().to_path_buf()).unwrap_err();
        assert!(matches!(err, ExvmsError::UnsupportedDatabase(_)));
    }

    fn create_empty_cloud_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZCLOUDRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZLOCALDURATION FLOAT,
                ZENCRYPTEDTITLE VARCHAR,
                ZPATH VARCHAR
            );
            "#,
        )
        .unwrap();
    }

    fn create_legacy_recordings_db(path: &Path, with_row: bool) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ZRECORDING (
                Z_PK INTEGER PRIMARY KEY,
                ZDATE TIMESTAMP,
                ZDURATION FLOAT,
                ZCUSTOMLABEL VARCHAR,
                ZPATH VARCHAR
            );
            "#,
        )
        .unwrap();
        if with_row {
            conn.execute(
                "INSERT INTO ZRECORDING (Z_PK, ZDATE, ZDURATION, ZCUSTOMLABEL, ZPATH) VALUES (1, 1.0, 1.0, 'One', 'one.m4a')",
                [],
            )
            .unwrap();
        }
    }
}
