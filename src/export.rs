use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use rayon::ThreadPoolBuilder;
use rayon::prelude::*;

use crate::db::{Recording, RecordingPathKind};
use crate::macos;
use crate::sanitize;
use crate::time;
use crate::{ExvmsError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    ExplicitIds,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Local,
    NeedsDownload(String),
    OutsideRecordingsDir(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedExport {
    pub id: i64,
    pub title: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub recorded_at: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedRecording {
    pub id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFailure {
    pub id: i64,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportReport {
    pub successes: Vec<PlannedExport>,
    pub skipped: Vec<SkippedRecording>,
    pub failures: Vec<ExportFailure>,
}

pub fn plan_exports(
    recordings_dir: &Path,
    output_dir: &Path,
    recordings: &[Recording],
    requested_ids: Option<&[i64]>,
    mode: SelectionMode,
) -> Result<(Vec<PlannedExport>, Vec<SkippedRecording>)> {
    let selected = select_recordings(recordings, requested_ids)?;
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    let mut needs_download_requested = Vec::new();

    for recording in selected {
        match ensure_exportable_availability(recordings_dir, recording)? {
            Availability::Local => {
                planned.push(plan_single(recordings_dir, output_dir, recording)?);
            }
            Availability::NeedsDownload(reason) => {
                if mode == SelectionMode::ExplicitIds {
                    needs_download_requested.push(recording.id);
                    continue;
                }
                skipped.push(SkippedRecording {
                    id: recording.id,
                    reason,
                });
            }
            Availability::OutsideRecordingsDir(path) => {
                return Err(ExvmsError::UnsafeSourcePath {
                    id: recording.id,
                    path,
                });
            }
        }
    }

    if !needs_download_requested.is_empty() {
        return Err(ExvmsError::NeedsDownloadRequested(needs_download_requested));
    }

    if planned.is_empty() && (mode == SelectionMode::ExplicitIds || skipped.is_empty()) {
        return Err(ExvmsError::NoExportableRecordings);
    }

    validate_targets(&planned)?;
    Ok((planned, skipped))
}

pub fn execute_exports(
    planned: Vec<PlannedExport>,
    skipped: Vec<SkippedRecording>,
    jobs: usize,
) -> ExportReport {
    let successes = Mutex::new(Vec::new());
    let failures = Mutex::new(Vec::new());
    let jobs = jobs.max(1);

    let pool = ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .expect("failed to build export thread pool");

    pool.install(|| {
        planned.par_iter().for_each(|item| match execute_one(item) {
            Ok(()) => successes.lock().unwrap().push(item.clone()),
            Err(err) => failures.lock().unwrap().push(ExportFailure {
                id: item.id,
                error: err.to_string(),
            }),
        });
    });

    let mut successes = successes.into_inner().unwrap();
    let mut failures = failures.into_inner().unwrap();
    successes.sort_by_key(|item| item.id);
    failures.sort_by_key(|item| item.id);

    ExportReport {
        successes,
        skipped,
        failures,
    }
}

fn execute_one(item: &PlannedExport) -> Result<()> {
    if let Some(parent) = item.target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&item.source, &item.target)?;
    macos::set_export_timestamps(
        &item.target,
        time::apple_seconds_to_system_time(item.recorded_at),
    )?;
    Ok(())
}

fn select_recordings<'a>(
    recordings: &'a [Recording],
    requested_ids: Option<&[i64]>,
) -> Result<Vec<&'a Recording>> {
    if let Some(ids) = requested_ids {
        let by_id = recordings
            .iter()
            .map(|row| (row.id, row))
            .collect::<HashMap<_, _>>();
        let mut missing = Vec::new();
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(*id) {
                continue;
            }
            match by_id.get(id) {
                Some(row) => selected.push(*row),
                None => missing.push(*id),
            }
        }
        if missing.is_empty() {
            Ok(selected)
        } else {
            Err(ExvmsError::MissingIds(missing))
        }
    } else {
        Ok(recordings.iter().collect())
    }
}

pub fn classify_availability(recordings_dir: &Path, recording: &Recording) -> Result<Availability> {
    classify_availability_inner(recordings_dir, recording, true)
}

pub fn ensure_exportable_availability(
    recordings_dir: &Path,
    recording: &Recording,
) -> Result<Availability> {
    let availability = classify_availability_inner(recordings_dir, recording, false)?;
    if let Availability::OutsideRecordingsDir(path) = &availability {
        return Err(ExvmsError::UnsafeSourcePath {
            id: recording.id,
            path: path.clone(),
        });
    }
    Ok(availability)
}

fn classify_availability_inner(
    recordings_dir: &Path,
    recording: &Recording,
    report_unsafe_as_status: bool,
) -> Result<Availability> {
    if recording.path_kind == RecordingPathKind::Missing {
        return Ok(Availability::NeedsDownload(
            "database path is empty; open/download it in Voice Memos.app first".to_string(),
        ));
    }

    let source = match resolve_source(recordings_dir, recording) {
        Ok(source) => source,
        Err(ExvmsError::UnsafeSourcePath { path, .. }) if report_unsafe_as_status => {
            return Ok(Availability::OutsideRecordingsDir(path));
        }
        Err(err) => return Err(err),
    };

    match source.metadata() {
        Ok(meta) if meta.len() > 0 => Ok(Availability::Local),
        Ok(_) => Ok(Availability::NeedsDownload(
            "local audio file is 0 bytes; open/download it in Voice Memos.app first".to_string(),
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Availability::NeedsDownload(
            "local audio file is missing; open/download it in Voice Memos.app first".to_string(),
        )),
        Err(err) => Err(ExvmsError::Availability {
            id: recording.id,
            message: err.to_string(),
        }),
    }
}

fn plan_single(
    recordings_dir: &Path,
    output_dir: &Path,
    recording: &Recording,
) -> Result<PlannedExport> {
    let source = resolve_source(recordings_dir, recording)?;
    let mut target_dir = output_dir.to_path_buf();
    if let Some(folder_name) = recording
        .folder_name
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        target_dir.push(sanitize::filename(folder_name));
    }

    let source_extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());
    let mut filename = sanitize::filename(&recording.title);
    if Path::new(&filename).extension().is_none()
        && let Some(extension) = source_extension
    {
        filename.push('.');
        filename.push_str(extension);
    }

    Ok(PlannedExport {
        id: recording.id,
        title: recording.title.clone(),
        source,
        target: target_dir.join(filename),
        recorded_at: recording.recorded_at,
    })
}

fn resolve_source(recordings_dir: &Path, recording: &Recording) -> Result<PathBuf> {
    match recording.path_kind {
        RecordingPathKind::Missing => Err(ExvmsError::UnsafeSourcePath {
            id: recording.id,
            path: recording.db_path.clone(),
        }),
        RecordingPathKind::Relative => {
            resolve_relative_source(recordings_dir, recording.id, &recording.db_path)
        }
        RecordingPathKind::Absolute => {
            resolve_legacy_absolute_source(recordings_dir, recording.id, &recording.db_path)
        }
    }
}

fn resolve_relative_source(recordings_dir: &Path, id: i64, raw_path: &str) -> Result<PathBuf> {
    let relative = Path::new(raw_path);
    if relative.is_absolute() || has_escape_component(relative) {
        return Err(ExvmsError::UnsafeSourcePath {
            id,
            path: raw_path.to_string(),
        });
    }
    Ok(recordings_dir.join(relative))
}

fn resolve_legacy_absolute_source(
    recordings_dir: &Path,
    id: i64,
    raw_path: &str,
) -> Result<PathBuf> {
    let original = Path::new(raw_path);
    if !original.is_absolute() {
        return resolve_relative_source(recordings_dir, id, raw_path);
    }

    if let Ok(relative) = original.strip_prefix(recordings_dir)
        && !has_escape_component(relative)
    {
        return Ok(recordings_dir.join(relative));
    }

    if let Some(relative) = relative_tail_after_recordings_dir(original, recordings_dir) {
        return Ok(recordings_dir.join(relative));
    }

    if let Some(file_name) = original.file_name() {
        let candidate = recordings_dir.join(file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(ExvmsError::UnsafeSourcePath {
        id,
        path: raw_path.to_string(),
    })
}

fn has_escape_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn relative_tail_after_recordings_dir(original: &Path, recordings_dir: &Path) -> Option<PathBuf> {
    let original_components = normal_components(original);
    let recordings_dir_components = normal_components(recordings_dir);

    if let (Some(original_user), Some(recordings_user)) = (
        user_component(&original_components),
        user_component(&recordings_dir_components),
    ) && original_user != recordings_user
    {
        return None;
    }

    for suffix in known_recordings_suffixes() {
        let Some(index) = find_subsequence(&original_components, suffix) else {
            continue;
        };
        let tail = &original_components[index + suffix.len()..];
        if tail.is_empty() {
            return None;
        }
        let relative = path_from_components(tail);
        if !has_escape_component(&relative) {
            return Some(relative);
        }
    }

    None
}

fn known_recordings_suffixes() -> &'static [&'static [&'static str]] {
    &[
        &[
            "Library",
            "Group Containers",
            "group.com.apple.VoiceMemos.shared",
            "Recordings",
        ],
        &[
            "Library",
            "Containers",
            "Voice Memos",
            "Data",
            "Library",
            "Application Support",
            "Recordings",
        ],
        &[
            "Library",
            "Application Support",
            "com.apple.voicememos",
            "Recordings",
        ],
    ]
}

fn normal_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_os_string()),
            _ => None,
        })
        .collect()
}

fn user_component(components: &[OsString]) -> Option<&OsString> {
    components
        .windows(2)
        .find(|window| window[0] == OsStr::new("Users"))
        .map(|window| &window[1])
}

fn find_subsequence(components: &[OsString], suffix: &[&str]) -> Option<usize> {
    components.windows(suffix.len()).position(|window| {
        window
            .iter()
            .zip(suffix)
            .all(|(component, expected)| component == OsStr::new(expected))
    })
}

fn path_from_components(components: &[OsString]) -> PathBuf {
    let mut path = PathBuf::new();
    for component in components {
        path.push(component);
    }
    path
}

fn validate_targets(planned: &[PlannedExport]) -> Result<()> {
    let mut by_target: HashMap<PathBuf, Vec<i64>> = HashMap::new();
    for item in planned {
        if item.target.exists() {
            return Err(ExvmsError::TargetExists {
                id: item.id,
                path: item.target.clone(),
            });
        }
        by_target
            .entry(item.target.clone())
            .or_default()
            .push(item.id);
    }

    for (path, ids) in by_target {
        if ids.len() > 1 {
            return Err(ExvmsError::TargetCollision { path, ids });
        }
    }
    Ok(())
}

pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get().min(4))
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::RecordingStoreKind;
    use std::io::Write;

    fn recording(id: i64, title: &str, rel: &str, folder: Option<&str>) -> Recording {
        let path_kind = if rel.is_empty() {
            RecordingPathKind::Missing
        } else if Path::new(rel).is_absolute() {
            RecordingPathKind::Absolute
        } else {
            RecordingPathKind::Relative
        };
        Recording {
            id,
            title: title.to_string(),
            recorded_at: id as f64,
            duration: 1.0,
            local_duration: 1.0,
            eviction_date: None,
            db_path: rel.to_string(),
            path_kind,
            store_kind: RecordingStoreKind::CloudRecordings,
            folder_id: folder.map(|_| 1),
            folder_name: folder.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn explicit_id_selection_fails_before_any_plan_when_id_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let rows = vec![recording(1, "One", "one.m4a", None)];
        let err = plan_exports(
            tmp.path(),
            tmp.path(),
            &rows,
            Some(&[1, 2]),
            SelectionMode::ExplicitIds,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::MissingIds(ids) if ids == vec![2]));
    }

    #[test]
    fn explicit_id_selection_fails_for_cloud_only_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::File::create(tmp.path().join("one.m4a")).unwrap();
        fs::File::create(tmp.path().join("two.m4a")).unwrap();
        let rows = vec![recording(1, "One", "one.m4a", None)];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            Some(&[1]),
            SelectionMode::ExplicitIds,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::NeedsDownloadRequested(ids) if ids == vec![1]));

        let rows = vec![
            recording(1, "One", "one.m4a", None),
            recording(2, "Two", "two.m4a", None),
        ];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            Some(&[1, 2]),
            SelectionMode::ExplicitIds,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::NeedsDownloadRequested(ids) if ids == vec![1, 2]));
    }

    #[test]
    fn all_selection_skips_cloud_only_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut local = fs::File::create(tmp.path().join("one.m4a")).unwrap();
        writeln!(local, "audio").unwrap();
        fs::File::create(tmp.path().join("two.m4a")).unwrap();
        let rows = vec![
            recording(1, "One", "one.m4a", None),
            recording(2, "Two", "two.m4a", None),
        ];
        let (planned, skipped) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(
            planned.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            skipped.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn all_selection_skips_empty_database_path() {
        let tmp = tempfile::tempdir().unwrap();
        let rows = vec![recording(1, "One", "", None)];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            Some(&[1]),
            SelectionMode::ExplicitIds,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::NeedsDownloadRequested(ids) if ids == vec![1]));

        let (planned, skipped) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert!(planned.is_empty());
        assert_eq!(
            skipped.iter().map(|row| row.id).collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn view_classification_reports_unsafe_path_without_error() {
        let tmp = tempfile::tempdir().unwrap();
        let row = recording(1, "One", "../one.m4a", None);
        let availability = classify_availability(tmp.path(), &row).unwrap();
        assert!(
            matches!(availability, Availability::OutsideRecordingsDir(path) if path == "../one.m4a")
        );
    }

    #[test]
    fn export_preflight_rejects_unsafe_path() {
        let tmp = tempfile::tempdir().unwrap();
        let rows = vec![recording(1, "One", "../one.m4a", None)];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::UnsafeSourcePath { .. }));
    }

    #[test]
    fn legacy_absolute_path_maps_to_backup_recordings_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let recordings_dir = tmp
            .path()
            .join("Volumes/Restore_iMac/Users/iXerol/Library/Application Support/com.apple.voicememos/Recordings");
        fs::create_dir_all(&recordings_dir).unwrap();
        let mut source = fs::File::create(recordings_dir.join("legacy.m4a")).unwrap();
        writeln!(source, "audio").unwrap();

        let row = recording(
            1,
            "Legacy",
            "/Users/iXerol/Library/Application Support/com.apple.voicememos/Recordings/legacy.m4a",
            None,
        );
        let (planned, skipped) = plan_exports(
            &recordings_dir,
            &tmp.path().join("out"),
            &[row],
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert!(skipped.is_empty());
        assert_eq!(planned[0].source, recordings_dir.join("legacy.m4a"));
    }

    #[test]
    fn legacy_absolute_path_can_fallback_to_existing_basename() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = fs::File::create(tmp.path().join("legacy.m4a")).unwrap();
        writeln!(source, "audio").unwrap();

        let row = recording(1, "Legacy", "/private/old/location/legacy.m4a", None);
        let (planned, skipped) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &[row],
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert!(skipped.is_empty());
        assert_eq!(planned[0].source, tmp.path().join("legacy.m4a"));
    }

    #[test]
    fn detects_duplicate_targets_after_sanitization() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.m4a", "two.m4a"] {
            let mut file = fs::File::create(tmp.path().join(name)).unwrap();
            writeln!(file, "audio").unwrap();
        }
        let rows = vec![
            recording(1, "A/B", "one.m4a", None),
            recording(2, "A:B", "two.m4a", None),
        ];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::TargetCollision { .. }));
    }

    #[test]
    fn detects_existing_target() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = fs::File::create(tmp.path().join("one.m4a")).unwrap();
        writeln!(source, "audio").unwrap();
        fs::create_dir(tmp.path().join("out")).unwrap();
        fs::File::create(tmp.path().join("out").join("One.m4a")).unwrap();
        let rows = vec![recording(1, "One", "one.m4a", None)];
        let err = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap_err();
        assert!(matches!(err, ExvmsError::TargetExists { .. }));
    }

    #[test]
    fn plans_folder_output() {
        let tmp = tempfile::tempdir().unwrap();
        let mut source = fs::File::create(tmp.path().join("one.m4a")).unwrap();
        writeln!(source, "audio").unwrap();
        let rows = vec![recording(1, "One", "one.m4a", Some("Folder/Name"))];
        let (planned, _) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(
            planned[0].target,
            tmp.path().join("out").join("Folder_Name").join("One.m4a")
        );
    }
}
