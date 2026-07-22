use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use std::time::{Duration, Instant};

use filetime::FileTime;

use crate::Result;

const AUDIO_DURATION_TIMEOUT: Duration = Duration::from_secs(5);
const BOX_HEADER_SIZE: u64 = 8;
const EXTENDED_BOX_HEADER_SIZE: u64 = 16;
static AFINFO_OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn set_export_timestamps(path: &Path, timestamp: SystemTime) -> Result<()> {
    filetime::set_file_mtime(path, FileTime::from_system_time(timestamp))?;
    set_creation_time(path, timestamp)?;
    Ok(())
}

pub fn audio_duration_seconds(path: &Path) -> Result<f64> {
    if let Ok(Some(seconds)) = iso_media_duration_seconds(path) {
        return Ok(seconds);
    }
    let stdout_path = temporary_afinfo_output_path("stdout");
    let stderr_path = temporary_afinfo_output_path("stderr");
    let stdout_file = std::fs::File::create(&stdout_path).map_err(|err| {
        crate::ExvmsError::Macos(format!("failed to create afinfo stdout capture: {err}"))
    })?;
    let stderr_file = std::fs::File::create(&stderr_path).map_err(|err| {
        crate::ExvmsError::Macos(format!("failed to create afinfo stderr capture: {err}"))
    })?;

    let mut child = std::process::Command::new("/usr/bin/afinfo")
        .arg(path)
        .stdout(std::process::Stdio::from(stdout_file))
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()
        .map_err(|err| crate::ExvmsError::Macos(format!("failed to run afinfo: {err}")))?;

    let started_at = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started_at.elapsed() < AUDIO_DURATION_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&stdout_path);
                let _ = std::fs::remove_file(&stderr_path);
                return Err(crate::ExvmsError::Macos(format!(
                    "afinfo timed out after {} seconds for {}",
                    AUDIO_DURATION_TIMEOUT.as_secs(),
                    path.display()
                )));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&stdout_path);
                let _ = std::fs::remove_file(&stderr_path);
                return Err(crate::ExvmsError::Macos(format!(
                    "failed waiting for afinfo: {err}"
                )));
            }
        }
    }

    let status = child
        .wait()
        .map_err(|err| crate::ExvmsError::Macos(format!("failed waiting for afinfo: {err}")))?;
    let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
    let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
    let _ = std::fs::remove_file(&stdout_path);
    let _ = std::fs::remove_file(&stderr_path);

    if !status.success() {
        let stderr = stderr.trim().to_string();
        return Err(crate::ExvmsError::Macos(format!(
            "afinfo exited with status {}: {}",
            status, stderr
        )));
    }

    for line in stdout.lines() {
        let value = line.trim();
        if let Some(raw) = value.strip_prefix("estimated duration:") {
            let number = raw.trim().strip_suffix("sec").unwrap_or(raw.trim()).trim();
            let seconds = number.parse::<f64>().map_err(|err| {
                crate::ExvmsError::Macos(format!(
                    "failed to parse afinfo duration `{number}`: {err}"
                ))
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

fn temporary_afinfo_output_path(label: &str) -> std::path::PathBuf {
    let counter = AFINFO_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "exvms-afinfo-{}-{counter}-{label}.txt",
        std::process::id()
    ))
}

fn iso_media_duration_seconds(path: &Path) -> std::io::Result<Option<f64>> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    find_mvhd_duration(&mut file, len)
}

fn find_mvhd_duration<R: Read + Seek>(reader: &mut R, end: u64) -> std::io::Result<Option<f64>> {
    let mut offset = reader.stream_position()?;
    while offset + BOX_HEADER_SIZE <= end {
        reader.seek(SeekFrom::Start(offset))?;
        let Some(header) = read_box_header(reader, offset, end)? else {
            return Ok(None);
        };

        let payload_start = offset + header.header_size;
        let payload_end = offset + header.size;
        if payload_end > end || payload_start > payload_end {
            return Ok(None);
        }

        match &header.kind {
            b"moov" => {
                reader.seek(SeekFrom::Start(payload_start))?;
                if let Some(duration) = find_mvhd_duration(reader, payload_end)? {
                    return Ok(Some(duration));
                }
            }
            b"mvhd" => {
                reader.seek(SeekFrom::Start(payload_start))?;
                return parse_mvhd_duration(reader, payload_end - payload_start);
            }
            _ => {}
        }

        if header.size == 0 {
            return Ok(None);
        }
        offset += header.size;
    }
    Ok(None)
}

struct BoxHeader {
    size: u64,
    header_size: u64,
    kind: [u8; 4],
}

fn read_box_header<R: Read>(
    reader: &mut R,
    offset: u64,
    end: u64,
) -> std::io::Result<Option<BoxHeader>> {
    let mut header = [0_u8; BOX_HEADER_SIZE as usize];
    if reader.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let mut kind = [0_u8; 4];
    kind.copy_from_slice(&header[4..8]);
    let short_size = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
    let (size, header_size) = match short_size {
        0 => (end - offset, BOX_HEADER_SIZE),
        1 => {
            let mut extended = [0_u8; 8];
            reader.read_exact(&mut extended)?;
            (u64::from_be_bytes(extended), EXTENDED_BOX_HEADER_SIZE)
        }
        value => (value, BOX_HEADER_SIZE),
    };
    if size < header_size {
        return Ok(None);
    }
    Ok(Some(BoxHeader {
        size,
        header_size,
        kind,
    }))
}

fn parse_mvhd_duration<R: Read>(reader: &mut R, payload_size: u64) -> std::io::Result<Option<f64>> {
    let mut version_and_flags = [0_u8; 4];
    reader.read_exact(&mut version_and_flags)?;
    let version = version_and_flags[0];

    match version {
        0 if payload_size >= 20 => {
            let _creation_time = read_u32(reader)?;
            let _modification_time = read_u32(reader)?;
            duration_from_timescale(read_u32(reader)? as u64, read_u32(reader)? as u64)
        }
        1 if payload_size >= 32 => {
            let _creation_time = read_u64(reader)?;
            let _modification_time = read_u64(reader)?;
            duration_from_timescale(read_u32(reader)? as u64, read_u64(reader)?)
        }
        _ => Ok(None),
    }
}

fn duration_from_timescale(timescale: u64, duration: u64) -> std::io::Result<Option<f64>> {
    if timescale == 0 || duration == 0 {
        return Ok(None);
    }
    let seconds = duration as f64 / timescale as f64;
    Ok((seconds.is_finite() && seconds > 0.0).then_some(seconds))
}

fn read_u32<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn read_u64<R: Read>(reader: &mut R) -> std::io::Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn iso_media_duration_reads_movie_header_duration() {
        let path =
            std::env::temp_dir().join(format!("exvms-mvhd-duration-{}.mov", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&mp4_box(*b"ftyp", b"qt  \0\0\0\0qt  "))
            .unwrap();
        file.write_all(&mp4_box(
            *b"moov",
            &mp4_box(*b"mvhd", &mvhd_payload(1_000, 2_500)),
        ))
        .unwrap();
        drop(file);

        let duration = iso_media_duration_seconds(&path).unwrap().unwrap();
        let _ = std::fs::remove_file(path);

        assert!((duration - 2.5).abs() < f64::EPSILON);
    }

    fn mp4_box(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut output = Vec::with_capacity(size as usize);
        output.extend_from_slice(&size.to_be_bytes());
        output.extend_from_slice(&kind);
        output.extend_from_slice(payload);
        output
    }

    fn mvhd_payload(timescale: u32, duration: u32) -> Vec<u8> {
        let mut output = Vec::new();
        output.extend_from_slice(&[0, 0, 0, 0]);
        output.extend_from_slice(&0_u32.to_be_bytes());
        output.extend_from_slice(&0_u32.to_be_bytes());
        output.extend_from_slice(&timescale.to_be_bytes());
        output.extend_from_slice(&duration.to_be_bytes());
        output
    }
}
