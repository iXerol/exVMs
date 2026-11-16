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
use crate::storage::RECORDINGS_DIRECTORY_SUFFIXES;
use crate::template::FilenameTemplate;
use crate::time;
use crate::{ExvmsError, Result};
use chrono::Datelike;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    ExplicitIds,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportGrouping(u32);

impl ExportGrouping {
    pub const FOLDER: Self = Self(1 << 0);
    pub const DATE: Self = Self(1 << 1);

    pub fn from_bits(bits: u32) -> Result<Self> {
        if bits & !(Self::FOLDER.0 | Self::DATE.0) == 0 {
            Ok(Self(bits))
        } else {
            Err(ExvmsError::InvalidArguments(format!(
                "invalid export folder option bits: {bits}"
            )))
        }
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl Default for ExportGrouping {
    fn default() -> Self {
        Self::FOLDER
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    InRecordingsDirectory,
    NeedsDownload(String),
    Corrupted(String),
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
    let template = FilenameTemplate::default_template();
    plan_exports_with_template(
        recordings_dir,
        output_dir,
        recordings,
        requested_ids,
        mode,
        &template,
    )
}

pub fn plan_exports_with_template(
    recordings_dir: &Path,
    output_dir: &Path,
    recordings: &[Recording],
    requested_ids: Option<&[i64]>,
    mode: SelectionMode,
    template: &FilenameTemplate,
) -> Result<(Vec<PlannedExport>, Vec<SkippedRecording>)> {
    plan_exports_with_template_and_grouping(
        recordings_dir,
        output_dir,
        recordings,
        requested_ids,
        mode,
        template,
        ExportGrouping::default(),
    )
}

pub fn plan_exports_with_template_and_grouping(
    recordings_dir: &Path,
    output_dir: &Path,
    recordings: &[Recording],
    requested_ids: Option<&[i64]>,
    mode: SelectionMode,
    template: &FilenameTemplate,
    grouping: ExportGrouping,
) -> Result<(Vec<PlannedExport>, Vec<SkippedRecording>)> {
    let selected = select_recordings(recordings, requested_ids)?;
    let mut planned = Vec::new();
    let mut skipped = Vec::new();
    let mut unavailable_requested = Vec::new();

    for recording in selected {
        match ensure_exportable_availability(recordings_dir, recording)? {
            Availability::InRecordingsDirectory => {
                planned.push(plan_single(
                    recordings_dir,
                    output_dir,
                    recording,
                    template,
                    grouping,
                )?);
            }
            Availability::NeedsDownload(reason) => {
                if mode == SelectionMode::ExplicitIds {
                    unavailable_requested.push(recording.id);
                    continue;
                }
                skipped.push(SkippedRecording {
                    id: recording.id,
                    reason,
                });
            }
            Availability::Corrupted(reason) => {
                if mode == SelectionMode::ExplicitIds {
                    unavailable_requested.push(recording.id);
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

    if !unavailable_requested.is_empty() {
        return Err(ExvmsError::NeedsDownloadRequested(unavailable_requested));
    }

    if planned.is_empty() && (mode == SelectionMode::ExplicitIds || skipped.is_empty()) {
        return Err(ExvmsError::NoExportableRecordings);
    }

    disambiguate_targets(&mut planned);

    validate_targets(&planned)?;
    Ok((planned, skipped))
}

pub fn execute_exports(
    planned: Vec<PlannedExport>,
    skipped: Vec<SkippedRecording>,
    jobs: usize,
) -> ExportReport {
    execute_exports_with_cancel(planned, skipped, jobs, || false)
}

pub fn execute_exports_with_cancel<F>(
    planned: Vec<PlannedExport>,
    skipped: Vec<SkippedRecording>,
    jobs: usize,
    is_cancelled: F,
) -> ExportReport
where
    F: Fn() -> bool + Sync,
{
    let successes = Mutex::new(Vec::new());
    let failures = Mutex::new(Vec::new());
    let jobs = jobs.max(1);

    let pool = ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .expect("failed to build export thread pool");

    pool.install(|| {
        planned.par_iter().for_each(|item| {
            if is_cancelled() {
                return;
            }
            match execute_one(item) {
                Ok(()) => successes.lock().unwrap().push(item.clone()),
                Err(err) => failures.lock().unwrap().push(ExportFailure {
                    id: item.id,
                    error: err.to_string(),
                }),
            }
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
    if let Err(err) = macos::set_export_timestamps(
        &item.target,
        time::apple_seconds_to_system_time(item.recorded_at),
    ) {
        eprintln!(
            "exvms export: copied id {} but failed to set timestamps on {}: {}",
            item.id,
            item.target.display(),
            err
        );
    }
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

    if recording.local_duration <= 0.0 {
        return Ok(Availability::NeedsDownload(
            "local duration is 0; open/download it in Voice Memos.app first".to_string(),
        ));
    }

    match source.metadata() {
        Ok(meta) if meta.len() > 0 => {
            if let Err(reason) =
                validate_audio_container_and_duration(&source, recording.local_duration)
            {
                return Ok(Availability::Corrupted(reason));
            }
            Ok(Availability::InRecordingsDirectory)
        }
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

fn validate_audio_container_and_duration(
    path: &Path,
    expected_duration: f64,
) -> std::result::Result<(), String> {
    let actual_duration = match macos::audio_duration_seconds(path) {
        Ok(seconds) => seconds,
        Err(err) => {
            return Err(format!("failed to read audio container duration: {err}"));
        }
    };

    if !duration_matches(expected_duration, actual_duration) {
        return Err(format!(
            "audio duration mismatch: database={expected_duration:.3}s, file={actual_duration:.3}s"
        ));
    }

    Ok(())
}

fn duration_matches(expected: f64, actual: f64) -> bool {
    const DURATION_TOLERANCE_SECONDS: f64 = 5.0;
    (expected - actual).abs() <= DURATION_TOLERANCE_SECONDS
}

fn plan_single(
    recordings_dir: &Path,
    output_dir: &Path,
    recording: &Recording,
    template: &FilenameTemplate,
    grouping: ExportGrouping,
) -> Result<PlannedExport> {
    let source = resolve_source(recordings_dir, recording)?;
    let recorded_at = time::apple_local_fixed_offset_datetime(recording.recorded_at);
    let mut target_dir = output_dir.to_path_buf();
    if grouping.contains(ExportGrouping::FOLDER) {
        if let Some(folder_name) = recording
            .folder_name
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            target_dir.push(sanitize::filename(folder_name));
        } else if grouping.contains(ExportGrouping::DATE) {
            target_dir.push("No Folder");
        }
    }
    if grouping.contains(ExportGrouping::DATE) {
        target_dir.push(date_directory_name(recorded_at));
    }

    let source_extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty());
    let rendered = template.render_stem(recording.id, &recording.title, recorded_at);
    let filename = finalize_filename(&rendered, source_extension);

    Ok(PlannedExport {
        id: recording.id,
        title: recording.title.clone(),
        source,
        target: target_dir.join(filename),
        recorded_at: recording.recorded_at,
    })
}

fn date_directory_name(recorded_at: chrono::DateTime<chrono::FixedOffset>) -> String {
    format!(
        "{}-{}-{}",
        recorded_at.year(),
        recorded_at.month(),
        recorded_at.day()
    )
}

fn finalize_filename(stem: &str, source_extension: Option<&str>) -> String {
    let mut filename = sanitize_stem(stem);
    if let Some(extension) = source_extension
        && !has_matching_extension(&filename, extension)
    {
        filename.push('.');
        filename.push_str(extension);
    }
    if source_extension.is_none() {
        filename = filename.trim_end_matches('.').trim().to_string();
        if filename.is_empty() {
            return "Untitled".to_string();
        }
    }
    filename
}

fn sanitize_stem(stem: &str) -> String {
    let sanitized = sanitize::filename(stem);
    let trimmed = sanitized.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn has_matching_extension(filename: &str, extension: &str) -> bool {
    let expected = format!(".{}", extension.to_ascii_lowercase());
    filename.to_ascii_lowercase().ends_with(&expected)
}

fn disambiguate_targets(planned: &mut [PlannedExport]) {
    let mut reserved = HashSet::new();
    for item in planned {
        let unique = next_available_target(&item.target, &mut reserved);
        item.target = unique;
    }
}

fn next_available_target(path: &Path, reserved: &mut HashSet<PathBuf>) -> PathBuf {
    if is_target_available(path, reserved) {
        let output = path.to_path_buf();
        reserved.insert(output.clone());
        return output;
    }

    for suffix in 2.. {
        let candidate = append_numeric_suffix(path, suffix);
        if is_target_available(&candidate, reserved) {
            reserved.insert(candidate.clone());
            return candidate;
        }
    }
    unreachable!("numeric suffix generator must eventually find an available path")
}

fn is_target_available(path: &Path, reserved: &HashSet<PathBuf>) -> bool {
    !reserved.contains(path) && !path.exists()
}

fn append_numeric_suffix(path: &Path, suffix: usize) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let new_name = match (
        path.file_stem().and_then(|value| value.to_str()),
        path.extension().and_then(|value| value.to_str()),
    ) {
        (Some(stem), Some(extension)) if !extension.is_empty() => {
            format!("{stem} {suffix}.{extension}")
        }
        (Some(stem), _) => format!("{stem} {suffix}"),
        _ => format!("{file_name} {suffix}"),
    };
    parent.join(new_name)
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

    for suffix in RECORDINGS_DIRECTORY_SUFFIXES {
        let suffix_components: Vec<_> = Path::new(suffix).iter().collect();
        let Some(index) = find_subsequence(&original_components, &suffix_components) else {
            continue;
        };
        let tail = &original_components[index + suffix_components.len()..];
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

fn find_subsequence(components: &[OsString], suffix: &[&OsStr]) -> Option<usize> {
    components.windows(suffix.len()).position(|window| {
        window
            .iter()
            .zip(suffix)
            .all(|(component, expected)| component == *expected)
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
    use crate::template::FilenameTemplate;
    use hound::{SampleFormat, WavSpec, WavWriter};
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
            is_deleted: false,
            db_path: rel.to_string(),
            path_kind,
            store_kind: RecordingStoreKind::CloudRecordings,
            folder_id: folder.map(|_| 1),
            folder_name: folder.map(ToOwned::to_owned),
        }
    }

    fn recording_with_local_duration(
        id: i64,
        title: &str,
        rel: &str,
        folder: Option<&str>,
        local_duration: f64,
    ) -> Recording {
        let mut row = recording(id, title, rel, folder);
        row.local_duration = local_duration;
        row
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
        create_valid_wav(&tmp.path().join("one.wav"));
        fs::File::create(tmp.path().join("two.m4a")).unwrap();
        let rows = vec![
            recording(1, "One", "one.wav", None),
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
            .join("Volumes/Restore_iMac/Users/example/Library/Application Support/com.apple.voicememos/Recordings");
        fs::create_dir_all(&recordings_dir).unwrap();
        create_valid_wav(&recordings_dir.join("legacy.wav"));

        let row = recording(
            1,
            "Legacy",
            "/Users/example/Library/Application Support/com.apple.voicememos/Recordings/legacy.wav",
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
        assert_eq!(planned[0].source, recordings_dir.join("legacy.wav"));
    }

    #[test]
    fn sandbox_absolute_path_maps_nested_recording_to_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let sandbox_suffix =
            "Library/Containers/com.apple.VoiceMemos/Data/Library/Application Support/Recordings";
        let recordings_dir = tmp
            .path()
            .join("Volumes/Backup/Users/example")
            .join(sandbox_suffix);
        let relative = "Folder/memo.wav";
        fs::create_dir_all(recordings_dir.join("Folder")).unwrap();
        let source = recordings_dir.join(relative);
        create_valid_wav(&source);
        let row = recording(
            1,
            "Memo",
            &format!("/Users/example/{sandbox_suffix}/{relative}"),
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
        assert_eq!(planned[0].source, source);
    }

    #[test]
    fn classify_invalid_audio_file_as_corrupted() {
        let tmp = tempfile::tempdir().unwrap();
        let mut invalid = fs::File::create(tmp.path().join("broken.m4a")).unwrap();
        writeln!(invalid, "not audio").unwrap();
        let row = recording(1, "Broken", "broken.m4a", None);
        let availability = classify_availability(tmp.path(), &row).unwrap();
        assert!(matches!(availability, Availability::Corrupted(_)));
    }

    #[test]
    fn classify_duration_mismatch_file_as_corrupted() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let row = recording_with_local_duration(1, "One", "one.wav", None, 30.0);
        let availability = classify_availability(tmp.path(), &row).unwrap();
        assert!(matches!(availability, Availability::Corrupted(_)));
    }

    fn create_valid_wav(path: &Path) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for _ in 0..8_000 {
            writer.write_sample::<i16>(0).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn legacy_absolute_path_can_fallback_to_existing_basename() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("legacy.wav"));

        let row = recording(1, "Legacy", "/private/old/location/legacy.wav", None);
        let (planned, skipped) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &[row],
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert!(skipped.is_empty());
        assert_eq!(planned[0].source, tmp.path().join("legacy.wav"));
    }

    #[test]
    fn default_template_uses_macos_numeric_suffix_on_collision() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.wav", "two.wav"] {
            create_valid_wav(&tmp.path().join(name));
        }
        let rows = vec![
            recording(1, "A/B", "one.wav", None),
            recording(2, "A:B", "two.wav", None),
        ];
        let (planned, _) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(
            planned
                .iter()
                .map(|row| row.target.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["A_B.wav", "A_B 2.wav"]
        );
    }

    #[test]
    fn default_template_increments_numeric_suffix_for_repeated_batch_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.wav", "two.wav", "three.wav"] {
            create_valid_wav(&tmp.path().join(name));
        }
        let rows = vec![
            recording(1, "Title", "one.wav", None),
            recording(2, "Title", "two.wav", None),
            recording(3, "Title", "three.wav", None),
        ];
        let (planned, _) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(
            planned
                .iter()
                .map(|row| row.target.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Title.wav", "Title 2.wav", "Title 3.wav"]
        );
    }

    #[test]
    fn existing_target_gets_macos_numeric_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        fs::create_dir(tmp.path().join("out")).unwrap();
        fs::File::create(tmp.path().join("out").join("One.wav")).unwrap();
        let rows = vec![recording(1, "One", "one.wav", None)];
        let (planned, _) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(planned[0].target, tmp.path().join("out").join("One 2.wav"));
    }

    #[test]
    fn existing_targets_increment_numeric_suffix_until_available() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        fs::create_dir(tmp.path().join("out")).unwrap();
        for suffix in 1..10 {
            let name = if suffix == 1 {
                "Title.wav".to_string()
            } else {
                format!("Title {suffix}.wav")
            };
            fs::File::create(tmp.path().join("out").join(name)).unwrap();
        }
        let rows = vec![recording(1, "Title", "one.wav", None)];
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
            tmp.path().join("out").join("Title 10.wav")
        );
    }

    #[test]
    fn plans_folder_output() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let rows = vec![recording(1, "One", "one.wav", Some("Folder/Name"))];
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
            tmp.path().join("out").join("Folder_Name").join("One.wav")
        );
    }

    #[test]
    fn plans_date_output_when_date_grouping_is_selected() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let rows = vec![recording(1, "One", "one.wav", Some("Folder"))];
        let (planned, _) = plan_exports_with_template_and_grouping(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &FilenameTemplate::default_template(),
            ExportGrouping::DATE,
        )
        .unwrap();
        let date_dir = date_directory_name(time::apple_local_fixed_offset_datetime(1.0));
        assert_eq!(
            planned[0].target,
            tmp.path().join("out").join(date_dir).join("One.wav")
        );
    }

    #[test]
    fn plans_folder_then_date_output_and_uses_no_folder_for_unfiled_recordings() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.wav", "two.wav"] {
            create_valid_wav(&tmp.path().join(name));
        }
        let rows = vec![
            recording(1, "One", "one.wav", None),
            recording(2, "Two", "two.wav", Some("Live")),
        ];
        let grouping = ExportGrouping::FOLDER.union(ExportGrouping::DATE);
        let (planned, _) = plan_exports_with_template_and_grouping(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &FilenameTemplate::default_template(),
            grouping,
        )
        .unwrap();
        let first_date_dir = date_directory_name(time::apple_local_fixed_offset_datetime(1.0));
        let second_date_dir = date_directory_name(time::apple_local_fixed_offset_datetime(2.0));
        assert_eq!(
            planned[0].target,
            tmp.path()
                .join("out")
                .join("No Folder")
                .join(first_date_dir)
                .join("One.wav")
        );
        assert_eq!(
            planned[1].target,
            tmp.path()
                .join("out")
                .join("Live")
                .join(second_date_dir)
                .join("Two.wav")
        );
    }

    #[test]
    fn default_template_keeps_existing_filename_behavior() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let rows = vec![recording(1, "One", "one.wav", None)];
        let (planned, _) = plan_exports(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
        )
        .unwrap();
        assert_eq!(planned[0].target, tmp.path().join("out").join("One.wav"));
    }

    #[test]
    fn timestamp_failure_does_not_mark_copied_file_as_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("one.wav");
        let target = tmp.path().join("out").join("One.wav");
        create_valid_wav(&source);
        let planned = vec![PlannedExport {
            id: 1,
            title: "One".to_string(),
            source,
            target: target.clone(),
            recorded_at: -978_307_201.0,
        }];

        let report = execute_exports(planned, vec![], 1);

        assert_eq!(report.successes.len(), 1);
        assert!(report.failures.is_empty());
        assert!(target.exists());
    }

    #[test]
    fn custom_template_renders_expected_filename() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let template = FilenameTemplate::parse("{id} {title}").unwrap();
        let rows = vec![recording(1, "One", "one.wav", None)];
        let (planned, _) = plan_exports_with_template(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &template,
        )
        .unwrap();
        assert_eq!(planned[0].target, tmp.path().join("out").join("1 One.wav"));
    }

    #[test]
    fn custom_template_uses_macos_numeric_suffix_for_batch_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.wav", "two.wav"] {
            create_valid_wav(&tmp.path().join(name));
        }
        let template = FilenameTemplate::parse("{date:%Y-%m-%d}").unwrap();
        let rows = vec![
            recording(1, "One", "one.wav", None),
            recording(2, "Two", "two.wav", None),
        ];
        let (planned, _) = plan_exports_with_template(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &template,
        )
        .unwrap();
        let targets = planned
            .iter()
            .map(|row| {
                row.target
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, vec!["2001-01-01.wav", "2001-01-01 2.wav"]);
    }

    #[test]
    fn custom_template_with_id_still_disambiguates_if_collision_occurs() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["one.wav", "two.wav"] {
            create_valid_wav(&tmp.path().join(name));
        }
        let template = FilenameTemplate::parse("{id}").unwrap();
        let rows = vec![
            recording(1, "One", "one.wav", None),
            recording(1, "Two", "two.wav", None),
        ];
        let (planned, _) = plan_exports_with_template(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &template,
        )
        .unwrap();
        let targets = planned
            .iter()
            .map(|row| {
                row.target
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, vec!["1.wav", "1 2.wav"]);
    }

    #[test]
    fn template_filename_does_not_duplicate_extension() {
        let tmp = tempfile::tempdir().unwrap();
        create_valid_wav(&tmp.path().join("one.wav"));
        let template = FilenameTemplate::parse("{title}.wav").unwrap();
        let rows = vec![recording(1, "One", "one.wav", None)];
        let (planned, _) = plan_exports_with_template(
            tmp.path(),
            &tmp.path().join("out"),
            &rows,
            None,
            SelectionMode::All,
            &template,
        )
        .unwrap();
        assert_eq!(planned[0].target, tmp.path().join("out").join("One.wav"));
    }
}
