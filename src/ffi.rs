use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::ptr;

use crate::db::{self, RecordingPathKind, RecordingStoreKind};
use crate::export::{self, Availability, SelectionMode};
use crate::storage;
use crate::template::FilenameTemplate;
use crate::{ExvmsError, Result};

const EXVMS_OK: i32 = 0;
const EXVMS_ERROR: i32 = 1;
const EXVMS_INVALID_ARGUMENT: i32 = 2;
const EXVMS_PANIC: i32 = 255;

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsStoreResult {
    pub code: i32,
    pub message: *mut c_char,
    pub recordings_dir: *mut c_char,
    pub db_path: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsRecording {
    pub id: i64,
    pub title: *mut c_char,
    pub recorded_at: f64,
    pub duration: f64,
    pub local_duration: f64,
    pub db_path: *mut c_char,
    pub folder_name: *mut c_char,
    pub store_kind: i32,
    pub path_kind: i32,
    pub availability: i32,
    pub availability_detail: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsListResult {
    pub code: i32,
    pub message: *mut c_char,
    pub recordings_dir: *mut c_char,
    pub db_path: *mut c_char,
    pub recordings: *mut ExvmsRecording,
    pub recordings_len: usize,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsExportSuccess {
    pub id: i64,
    pub target: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsExportSkipped {
    pub id: i64,
    pub reason: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsExportFailure {
    pub id: i64,
    pub error: *mut c_char,
}

#[repr(C)]
#[derive(Debug)]
pub struct ExvmsExportResult {
    pub code: i32,
    pub message: *mut c_char,
    pub successes: *mut ExvmsExportSuccess,
    pub successes_len: usize,
    pub skipped: *mut ExvmsExportSkipped,
    pub skipped_len: usize,
    pub failures: *mut ExvmsExportFailure,
    pub failures_len: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_discover_store() -> ExvmsStoreResult {
    guard_store_result(|| {
        let store = storage::discover()?;
        Ok(ExvmsStoreResult {
            code: EXVMS_OK,
            message: ptr::null_mut(),
            recordings_dir: path_to_c_string(&store.recordings_dir),
            db_path: path_to_c_string(&store.db_path),
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_list_recordings() -> ExvmsListResult {
    guard_list_result(|| {
        let store = storage::discover()?;
        let rows = db::load_recordings(&store.db_path)?;
        let mut recordings = Vec::with_capacity(rows.len());

        for row in rows {
            let (availability, availability_detail) =
                match export::classify_availability(&store.recordings_dir, &row)? {
                    Availability::Local => (0, None),
                    Availability::NeedsDownload(reason) => (1, Some(reason)),
                    Availability::OutsideRecordingsDir(path) => (2, Some(path)),
                };
            recordings.push(ExvmsRecording {
                id: row.id,
                title: string_to_c_string(row.title),
                recorded_at: row.recorded_at,
                duration: row.duration,
                local_duration: row.local_duration,
                db_path: string_to_c_string(row.db_path),
                folder_name: optional_string_to_c_string(row.folder_name),
                store_kind: match row.store_kind {
                    RecordingStoreKind::CloudRecordings => 0,
                    RecordingStoreKind::LocalRecordings => 1,
                },
                path_kind: match row.path_kind {
                    RecordingPathKind::Relative => 0,
                    RecordingPathKind::Absolute => 1,
                    RecordingPathKind::Missing => 2,
                },
                availability,
                availability_detail: optional_string_to_c_string(availability_detail),
            });
        }

        let (recordings, recordings_len) = leak_vec(recordings);
        Ok(ExvmsListResult {
            code: EXVMS_OK,
            message: ptr::null_mut(),
            recordings_dir: path_to_c_string(&store.recordings_dir),
            db_path: path_to_c_string(&store.db_path),
            recordings,
            recordings_len,
        })
    })
}

/// # Safety
///
/// `output_dir` and `filename_template`, when non-null, must point to valid
/// NUL-terminated UTF-8 strings for the duration of the call. `ids` must point
/// to `ids_len` contiguous `i64` values when `ids_len > 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exvms_export_recordings(
    output_dir: *const c_char,
    ids: *const i64,
    ids_len: usize,
    jobs: usize,
    filename_template: *const c_char,
) -> ExvmsExportResult {
    guard_export_result(|| {
        let output_dir = c_str_arg(output_dir, "output_dir")?;
        let ids = ids_arg(ids, ids_len)?;
        if ids.is_empty() {
            return Err(ExvmsError::InvalidArguments(
                "at least one recording id is required".to_string(),
            ));
        }

        let template = optional_template_arg(filename_template)?;
        export_recordings(&output_dir, ids, jobs, &template)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_free_string(value: *mut c_char) {
    free_c_string(value);
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_free_store_result(result: ExvmsStoreResult) {
    free_c_string(result.message);
    free_c_string(result.recordings_dir);
    free_c_string(result.db_path);
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_free_list_result(result: ExvmsListResult) {
    free_c_string(result.message);
    free_c_string(result.recordings_dir);
    free_c_string(result.db_path);
    if !result.recordings.is_null() && result.recordings_len > 0 {
        let recordings = unsafe {
            Vec::from_raw_parts(
                result.recordings,
                result.recordings_len,
                result.recordings_len,
            )
        };
        for row in recordings {
            free_recording(row);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn exvms_free_export_result(result: ExvmsExportResult) {
    free_c_string(result.message);
    if !result.successes.is_null() && result.successes_len > 0 {
        let successes = unsafe {
            Vec::from_raw_parts(result.successes, result.successes_len, result.successes_len)
        };
        for item in successes {
            free_c_string(item.target);
        }
    }
    if !result.skipped.is_null() && result.skipped_len > 0 {
        let skipped =
            unsafe { Vec::from_raw_parts(result.skipped, result.skipped_len, result.skipped_len) };
        for item in skipped {
            free_c_string(item.reason);
        }
    }
    if !result.failures.is_null() && result.failures_len > 0 {
        let failures = unsafe {
            Vec::from_raw_parts(result.failures, result.failures_len, result.failures_len)
        };
        for item in failures {
            free_c_string(item.error);
        }
    }
}

fn export_recordings(
    output_dir: &str,
    ids: &[i64],
    jobs: usize,
    template: &FilenameTemplate,
) -> Result<ExvmsExportResult> {
    let store = storage::discover()?;
    let rows = db::load_recordings(&store.db_path)?;
    let (planned, skipped) = export::plan_exports_with_template(
        &store.recordings_dir,
        Path::new(output_dir),
        &rows,
        Some(ids),
        SelectionMode::ExplicitIds,
        template,
    )?;
    let report = export::execute_exports(planned, skipped, jobs.max(1));

    let successes = report
        .successes
        .into_iter()
        .map(|item| ExvmsExportSuccess {
            id: item.id,
            target: path_to_c_string(&item.target),
        })
        .collect::<Vec<_>>();
    let skipped = report
        .skipped
        .into_iter()
        .map(|item| ExvmsExportSkipped {
            id: item.id,
            reason: string_to_c_string(item.reason),
        })
        .collect::<Vec<_>>();
    let failures = report
        .failures
        .into_iter()
        .map(|item| ExvmsExportFailure {
            id: item.id,
            error: string_to_c_string(item.error),
        })
        .collect::<Vec<_>>();
    let code = if failures.is_empty() {
        EXVMS_OK
    } else {
        EXVMS_ERROR
    };
    let message = if failures.is_empty() {
        ptr::null_mut()
    } else {
        string_to_c_string("one or more recordings failed to export".to_string())
    };
    let (successes, successes_len) = leak_vec(successes);
    let (skipped, skipped_len) = leak_vec(skipped);
    let (failures, failures_len) = leak_vec(failures);

    Ok(ExvmsExportResult {
        code,
        message,
        successes,
        successes_len,
        skipped,
        skipped_len,
        failures,
        failures_len,
    })
}

fn c_str_arg(ptr: *const c_char, name: &str) -> Result<String> {
    if ptr.is_null() {
        return Err(ExvmsError::InvalidArguments(format!("{name} is null")));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_string)
        .map_err(|_| ExvmsError::InvalidArguments(format!("{name} must be valid UTF-8")))
}

fn ids_arg<'a>(ptr: *const i64, len: usize) -> Result<&'a [i64]> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(ExvmsError::InvalidArguments(
            "ids is null while ids_len is non-zero".to_string(),
        ));
    }
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn optional_template_arg(ptr: *const c_char) -> Result<FilenameTemplate> {
    if ptr.is_null() {
        return Ok(FilenameTemplate::default_template());
    }
    FilenameTemplate::parse(&c_str_arg(ptr, "filename_template")?)
}

fn guard_store_result(run: impl FnOnce() -> Result<ExvmsStoreResult>) -> ExvmsStoreResult {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => store_error(error_code(&err), err.to_string()),
        Err(_) => store_error(
            EXVMS_PANIC,
            "Rust panic crossed the FFI boundary".to_string(),
        ),
    }
}

fn guard_list_result(run: impl FnOnce() -> Result<ExvmsListResult>) -> ExvmsListResult {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => list_error(error_code(&err), err.to_string()),
        Err(_) => list_error(
            EXVMS_PANIC,
            "Rust panic crossed the FFI boundary".to_string(),
        ),
    }
}

fn guard_export_result(run: impl FnOnce() -> Result<ExvmsExportResult>) -> ExvmsExportResult {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => export_error(error_code(&err), err.to_string()),
        Err(_) => export_error(
            EXVMS_PANIC,
            "Rust panic crossed the FFI boundary".to_string(),
        ),
    }
}

fn error_code(err: &ExvmsError) -> i32 {
    match err {
        ExvmsError::InvalidArguments(_) | ExvmsError::InvalidFilenameFormat(_) => {
            EXVMS_INVALID_ARGUMENT
        }
        _ => EXVMS_ERROR,
    }
}

fn store_error(code: i32, message: String) -> ExvmsStoreResult {
    ExvmsStoreResult {
        code,
        message: string_to_c_string(message),
        recordings_dir: ptr::null_mut(),
        db_path: ptr::null_mut(),
    }
}

fn list_error(code: i32, message: String) -> ExvmsListResult {
    ExvmsListResult {
        code,
        message: string_to_c_string(message),
        recordings_dir: ptr::null_mut(),
        db_path: ptr::null_mut(),
        recordings: ptr::null_mut(),
        recordings_len: 0,
    }
}

fn export_error(code: i32, message: String) -> ExvmsExportResult {
    ExvmsExportResult {
        code,
        message: string_to_c_string(message),
        successes: ptr::null_mut(),
        successes_len: 0,
        skipped: ptr::null_mut(),
        skipped_len: 0,
        failures: ptr::null_mut(),
        failures_len: 0,
    }
}

fn free_recording(row: ExvmsRecording) {
    free_c_string(row.title);
    free_c_string(row.db_path);
    free_c_string(row.folder_name);
    free_c_string(row.availability_detail);
}

fn leak_vec<T>(mut items: Vec<T>) -> (*mut T, usize) {
    let len = items.len();
    if len == 0 {
        return (ptr::null_mut(), 0);
    }
    let ptr = items.as_mut_ptr();
    std::mem::forget(items);
    (ptr, len)
}

fn path_to_c_string(path: &Path) -> *mut c_char {
    string_to_c_string(path.to_string_lossy().into_owned())
}

fn optional_string_to_c_string(value: Option<String>) -> *mut c_char {
    value.map_or(ptr::null_mut(), string_to_c_string)
}

fn string_to_c_string(value: String) -> *mut c_char {
    let sanitized = value.replace('\0', "\u{fffd}");
    CString::new(sanitized)
        .expect("interior NULs were replaced")
        .into_raw()
}

fn free_c_string(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_rejects_missing_ids_before_discovery() {
        let output = CString::new("/tmp/exvms-out").unwrap();
        let result =
            unsafe { exvms_export_recordings(output.as_ptr(), ptr::null(), 1, 1, ptr::null()) };
        assert_eq!(result.code, EXVMS_INVALID_ARGUMENT);
        assert!(!result.message.is_null());
        exvms_free_export_result(result);
    }

    #[test]
    fn export_rejects_invalid_template_before_discovery() {
        let output = CString::new("/tmp/exvms-out").unwrap();
        let template = CString::new("{unknown}").unwrap();
        let ids = [1_i64];
        let result = unsafe {
            exvms_export_recordings(
                output.as_ptr(),
                ids.as_ptr(),
                ids.len(),
                1,
                template.as_ptr(),
            )
        };
        assert_eq!(result.code, EXVMS_INVALID_ARGUMENT);
        assert!(!result.message.is_null());
        exvms_free_export_result(result);
    }
}
