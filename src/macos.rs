use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use filetime::FileTime;

use crate::Result;

pub fn set_export_timestamps(path: &Path, timestamp: SystemTime) -> Result<()> {
    filetime::set_file_mtime(path, FileTime::from_system_time(timestamp))?;
    set_creation_time(path, timestamp)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn audio_duration_seconds(path: &Path) -> Result<f64> {
    let output = Command::new("/usr/bin/afinfo")
        .arg(path)
        .output()
        .map_err(|err| crate::ExvmsError::Macos(format!("failed to run afinfo: {err}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(crate::ExvmsError::Macos(format!(
            "afinfo exited with status {}: {}",
            output.status,
            stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let value = line.trim();
        if let Some(raw) = value.strip_prefix("estimated duration:") {
            let number = raw
                .trim()
                .strip_suffix("sec")
                .unwrap_or(raw.trim())
                .trim();
            let seconds = number.parse::<f64>().map_err(|err| {
                crate::ExvmsError::Macos(format!("failed to parse afinfo duration `{number}`: {err}"))
            })?;
            if seconds.is_finite() && seconds > 0.0 {
                return Ok(seconds);
            }
        }
    }

    Err(crate::ExvmsError::Macos(
        "afinfo output did not contain estimated duration".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn audio_duration_seconds(_path: &Path) -> Result<f64> {
    Err(crate::ExvmsError::Macos(
        "audio duration check is only supported on macOS".to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn set_creation_time(path: &Path, timestamp: SystemTime) -> Result<()> {
    use std::ffi::CString;
    use std::mem;
    use std::os::unix::ffi::OsStrExt;

    let duration = timestamp
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| crate::ExvmsError::Macos(format!("timestamp before Unix epoch: {err}")))?;
    let timespec = libc::timespec {
        tv_sec: duration.as_secs() as libc::time_t,
        tv_nsec: duration.subsec_nanos() as libc::c_long,
    };
    let mut attrs: libc::attrlist = unsafe { mem::zeroed() };
    attrs.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    attrs.commonattr = libc::ATTR_CMN_CRTIME;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| crate::ExvmsError::Macos("path contains NUL byte".to_string()))?;
    let ret = unsafe {
        libc::setattrlist(
            path.as_ptr(),
            &mut attrs as *mut _ as *mut libc::c_void,
            &timespec as *const _ as *mut libc::c_void,
            mem::size_of_val(&timespec),
            0,
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(not(target_os = "macos"))]
fn set_creation_time(_path: &Path, _timestamp: SystemTime) -> Result<()> {
    Ok(())
}
