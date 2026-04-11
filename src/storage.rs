use std::path::PathBuf;

use crate::{ExvmsError, Result};

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
    for dir in candidate_recording_dirs(&home) {
        if dir.is_dir() {
            let db_path = dir.join("CloudRecordings.db");
            if db_path.is_file() {
                return Ok(VoiceMemosStore {
                    recordings_dir: dir,
                    db_path,
                });
            }
            return Err(ExvmsError::DatabaseNotFound(dir));
        }
    }
    Err(ExvmsError::StorageNotFound)
}

pub fn candidate_recording_dirs(home: &std::path::Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings"),
        home.join("Library/Containers/Voice Memos/Data/Library/Application Support/Recordings"),
        home.join("Library/Application Support/com.apple.voicememos/Recordings"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
