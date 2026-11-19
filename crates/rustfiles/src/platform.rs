//! Volume discovery, NTFS/USN IO, and known folders.
//!
//! The NTFS parsers themselves live in `rustfiles-core` and are tested on every
//! OS. This module is the only place that talks to a raw volume.

#[cfg(not(windows))]
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
#[cfg(windows)]
use std::sync::atomic::Ordering;

#[cfg(windows)]
use rustfiles_core::{
    apply_fixup, parse_boot_sector, parse_mft_stream_from, parse_usn_page, unnamed_data_runs,
};
use rustfiles_core::{RawFile, UsnRecord};

#[derive(Clone, Debug)]
pub struct DetectedVolume {
    pub mount: String,
    pub label: String,
    pub fs_name: String,
    pub is_ntfs: bool,
    pub case_insensitive: bool,
}

pub struct NtfsScan {
    pub files: Vec<RawFile>,
    pub journal_id: u64,
    pub next_usn: i64,
}

pub struct UsnCatchup {
    pub journal_id: u64,
    pub next_usn: i64,
    pub records: Vec<UsnRecord>,
}

#[derive(Clone, Debug)]
pub struct Places {
    pub home: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    pub downloads: PathBuf,
    pub pictures: PathBuf,
    pub music: PathBuf,
    pub videos: PathBuf,
}

pub fn places() -> Places {
    let home = home_dir();
    let places = Places {
        desktop: home.join("Desktop"),
        documents: home.join("Documents"),
        downloads: home.join("Downloads"),
        pictures: home.join("Pictures"),
        music: home.join("Music"),
        videos: home.join("Videos"),
        home,
    };
    #[cfg(unix)]
    {
        let mut places = places;
        apply_user_dirs(&mut places);
        places
    }
    #[cfg(not(unix))]
    {
        places
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("RustFiles");
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("rustfiles");
    }
    home_dir().join(".config").join("rustfiles")
}

pub fn cache_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("RustFiles");
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("rustfiles");
    }
    home_dir().join(".cache").join("rustfiles")
}

pub fn index_path() -> PathBuf {
    cache_dir().join("index.rfi")
}

pub fn open_path(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .arg(path)
            .creation_flags(0x0800_0000)
            .spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
}

pub fn detect_volumes() -> Vec<DetectedVolume> {
    if let Ok(spec) = std::env::var("RUSTFILES_INDEX_PATHS") {
        let volumes: Vec<_> = spec
            .split('|')
            .filter(|part| !part.is_empty())
            .map(volume_from_path)
            .collect();
        if !volumes.is_empty() {
            return volumes;
        }
    }
    detect_mounted_volumes()
}

pub fn volume_from_path(path: &str) -> DetectedVolume {
    DetectedVolume {
        mount: path.to_string(),
        label: Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        fs_name: "path".into(),
        is_ntfs: false,
        case_insensitive: cfg!(windows),
    }
}

pub fn scan_ntfs(mount: &str, progress: &AtomicU64) -> Result<NtfsScan, String> {
    #[cfg(windows)]
    {
        windows::scan_ntfs(mount, progress)
    }
    #[cfg(not(windows))]
    {
        let _ = (mount, progress);
        Err("NTFS MFT scan is only available on Windows".into())
    }
}

pub fn read_usn(mount: &str, journal_id: u64, start_usn: i64) -> Result<UsnCatchup, String> {
    #[cfg(windows)]
    {
        windows::read_usn(mount, journal_id, start_usn)
    }
    #[cfg(not(windows))]
    {
        let _ = (mount, journal_id, start_usn);
        Err("USN journal is only available on Windows".into())
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(unix)]
fn apply_user_dirs(places: &mut Places) {
    let file = places.home.join(".config").join("user-dirs.dirs");
    let Ok(text) = fs::read_to_string(file) else {
        return;
    };
    for line in text.lines() {
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let value = raw
            .trim()
            .trim_matches('"')
            .replace("$HOME", &places.home.display().to_string());
        let path = PathBuf::from(value);
        match key.trim() {
            "XDG_DESKTOP_DIR" => places.desktop = path,
            "XDG_DOCUMENTS_DIR" => places.documents = path,
            "XDG_DOWNLOAD_DIR" => places.downloads = path,
            "XDG_PICTURES_DIR" => places.pictures = path,
            "XDG_MUSIC_DIR" => places.music = path,
            "XDG_VIDEOS_DIR" => places.videos = path,
            _ => {}
        }
    }
}

#[cfg(not(windows))]
fn detect_mounted_volumes() -> Vec<DetectedVolume> {
    let mut volumes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(text) = fs::read_to_string("/proc/mounts") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let Some(_device) = parts.next() else {
                continue;
            };
            let Some(mount) = parts.next() else { continue };
            let Some(fs_name) = parts.next() else {
                continue;
            };
            let mount = unescape_mount(mount);
            if !real_filesystem(fs_name) || skip_mount(&mount) {
                continue;
            }
            if let Some(dev) = unix_device(Path::new(&mount)) {
                if !seen.insert(dev) {
                    continue;
                }
            }
            let insensitive = matches!(fs_name, "vfat" | "exfat" | "ntfs" | "ntfs3" | "fuseblk");
            volumes.push(DetectedVolume {
                label: fs_name.to_string(),
                fs_name: fs_name.to_string(),
                is_ntfs: fs_name.eq_ignore_ascii_case("ntfs") || fs_name == "ntfs3",
                case_insensitive: insensitive,
                mount,
            });
        }
    }
    if volumes.is_empty() {
        volumes.push(DetectedVolume {
            mount: "/".into(),
            label: "Root".into(),
            fs_name: "unknown".into(),
            is_ntfs: false,
            case_insensitive: false,
        });
    }
    volumes
}

#[cfg(not(windows))]
fn real_filesystem(name: &str) -> bool {
    matches!(
        name,
        "ext2"
            | "ext3"
            | "ext4"
            | "btrfs"
            | "xfs"
            | "ntfs"
            | "ntfs3"
            | "vfat"
            | "exfat"
            | "f2fs"
            | "zfs"
            | "bcachefs"
            | "jfs"
            | "fuseblk"
            | "overlay"
    )
}

#[cfg(not(windows))]
fn skip_mount(mount: &str) -> bool {
    mount.starts_with("/proc")
        || mount.starts_with("/sys")
        || mount.starts_with("/dev")
        || mount.starts_with("/run")
        || mount.starts_with("/snap")
}

#[cfg(unix)]
fn unix_device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|meta| meta.dev())
}

#[cfg(not(windows))]
fn unescape_mount(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

#[cfg(windows)]
fn detect_mounted_volumes() -> Vec<DetectedVolume> {
    windows::detect_volumes()
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, ReadFile,
        SetFilePointerEx, FILE_BEGIN, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00f4;
    const FSCTL_READ_USN_JOURNAL: u32 = 0x0009_00bb;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    struct VolumeHandle(HANDLE);

    impl Drop for VolumeHandle {
        fn drop(&mut self) {
            if self.0 != INVALID_HANDLE_VALUE {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub fn detect_volumes() -> Vec<DetectedVolume> {
        let mask = unsafe { GetLogicalDrives() };
        let mut volumes = Vec::new();
        for index in 0..26 {
            if mask & (1 << index) == 0 {
                continue;
            }
            let letter = (b'A' + index) as char;
            let mount = format!("{letter}:\\");
            let wide = wide(&mount);
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
                continue;
            }
            let mut label = [0u16; 64];
            let mut fs_name = [0u16; 32];
            let ok = unsafe {
                GetVolumeInformationW(
                    wide.as_ptr(),
                    label.as_mut_ptr(),
                    label.len() as u32,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    fs_name.as_mut_ptr(),
                    fs_name.len() as u32,
                )
            };
            let fs = if ok != 0 {
                utf16_buf(&fs_name)
            } else {
                String::new()
            };
            let label = if ok != 0 {
                utf16_buf(&label)
            } else {
                String::new()
            };
            volumes.push(DetectedVolume {
                mount,
                label: if label.is_empty() {
                    format!("{letter}:")
                } else {
                    label
                },
                is_ntfs: fs.eq_ignore_ascii_case("NTFS"),
                case_insensitive: true,
                fs_name: fs,
            });
        }
        volumes
    }

    pub fn scan_ntfs(mount: &str, progress: &AtomicU64) -> Result<NtfsScan, String> {
        let letter = mount
            .chars()
            .next()
            .filter(|ch| ch.is_ascii_alphabetic())
            .ok_or_else(|| format!("Not a drive letter: {mount}"))?;
        let handle = open_volume(&format!("\\\\.\\{letter}:"))?;
        let mut boot_bytes = vec![0u8; 4096];
        read_at(handle.0, 0, &mut boot_bytes)?;
        let boot = parse_boot_sector(&boot_bytes).map_err(|err| err.to_string())?;
        let record_size = boot.record_size as usize;
        let cluster = u64::from(boot.cluster_size);
        let mut first = vec![0u8; record_size];
        read_at(handle.0, boot.mft_lcn.saturating_mul(cluster), &mut first)?;
        apply_fixup(&mut first, boot.bytes_per_sector).map_err(|err| err.to_string())?;
        let runs = unnamed_data_runs(&first).map_err(|err| err.to_string())?;
        let mut files = Vec::new();
        let mut ordinal = 0u64;
        let mut carry: Vec<u8> = Vec::new();
        let chunk_clusters = ((8 * 1024 * 1024) / cluster.max(1)).max(1);
        for run in &runs {
            let mut left = run.clusters;
            let mut lcn = run.lcn;
            while left > 0 {
                let now = left.min(chunk_clusters);
                let bytes = (now.saturating_mul(cluster)) as usize;
                let mut buf = vec![0u8; bytes];
                if let Some(cluster_index) = lcn {
                    read_at(handle.0, cluster_index.saturating_mul(cluster), &mut buf)?;
                    lcn = Some(cluster_index.saturating_add(now));
                }
                left -= now;
                carry.extend_from_slice(&buf);
                let usable = carry.len() / record_size * record_size;
                if usable == 0 {
                    continue;
                }
                let parsed = parse_mft_stream_from(
                    record_size,
                    boot.bytes_per_sector,
                    &carry[..usable],
                    ordinal,
                )
                .map_err(|err| err.to_string())?;
                ordinal += (usable / record_size) as u64;
                progress.fetch_add(parsed.len() as u64, Ordering::Relaxed);
                files.extend(parsed);
                carry.drain(..usable);
            }
        }
        let (journal_id, next_usn) = query_journal(handle.0).unwrap_or((0, 0));
        Ok(NtfsScan {
            files,
            journal_id,
            next_usn,
        })
    }

    pub fn read_usn(mount: &str, journal_id: u64, start_usn: i64) -> Result<UsnCatchup, String> {
        let letter = mount
            .chars()
            .next()
            .filter(|ch| ch.is_ascii_alphabetic())
            .ok_or_else(|| format!("Not a drive letter: {mount}"))?;
        let handle = open_volume(&format!("\\\\.\\{letter}:"))?;
        let (current_id, _) = query_journal(handle.0)?;
        if current_id != journal_id {
            return Err("USN journal was recreated".into());
        }
        let mut records = Vec::new();
        let mut cursor = start_usn;
        let mut next_usn = start_usn;
        for _ in 0..10_000 {
            let mut input = [0u8; 40];
            input[0..8].copy_from_slice(&cursor.to_le_bytes());
            input[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            input[32..40].copy_from_slice(&journal_id.to_le_bytes());
            let mut output = vec![0u8; 64 * 1024];
            let mut returned = 0u32;
            let ok = unsafe {
                DeviceIoControl(
                    handle.0,
                    FSCTL_READ_USN_JOURNAL,
                    input.as_ptr() as *const c_void,
                    input.len() as u32,
                    output.as_mut_ptr() as *mut c_void,
                    output.len() as u32,
                    &mut returned,
                    null_mut(),
                )
            };
            if ok == 0 {
                let err = unsafe { GetLastError() };
                if records.is_empty() {
                    return Err(format!("USN read failed ({err})"));
                }
                break;
            }
            if returned < 8 {
                break;
            }
            output.truncate(returned as usize);
            let (next, page) = parse_usn_page(&output).map_err(|err| err.to_string())?;
            next_usn = next;
            if page.is_empty() || next == cursor {
                break;
            }
            cursor = next;
            records.extend(page);
            if records.len() > 2_000_000 {
                break;
            }
        }
        Ok(UsnCatchup {
            journal_id,
            next_usn,
            records,
        })
    }

    fn query_journal(handle: HANDLE) -> Result<(u64, i64), String> {
        let mut output = [0u8; 64];
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_USN_JOURNAL,
                null_mut(),
                0,
                output.as_mut_ptr() as *mut c_void,
                56,
                &mut returned,
                null_mut(),
            )
        };
        if ok == 0 || returned < 24 {
            return Err(format!("USN journal is not available ({})", unsafe {
                GetLastError()
            }));
        }
        let journal_id = u64::from_le_bytes(output[0..8].try_into().unwrap());
        let next_usn = i64::from_le_bytes(output[16..24].try_into().unwrap());
        Ok((journal_id, next_usn))
    }

    fn open_volume(path: &str) -> Result<VolumeHandle, String> {
        let wide = wide(path);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("Cannot open {path} ({})", unsafe {
                GetLastError()
            }));
        }
        Ok(VolumeHandle(handle))
    }

    fn read_at(handle: HANDLE, mut offset: u64, buf: &mut [u8]) -> Result<(), String> {
        let mut filled = 0;
        while filled < buf.len() {
            let mut new_pos = 0i64;
            let moved =
                unsafe { SetFilePointerEx(handle, offset as i64, &mut new_pos, FILE_BEGIN) };
            if moved == 0 {
                return Err(format!("seek failed ({})", unsafe { GetLastError() }));
            }
            let mut read = 0u32;
            let want = ((buf.len() - filled).min(1024 * 1024)) as u32;
            let ok = unsafe {
                ReadFile(
                    handle,
                    buf[filled..].as_mut_ptr(),
                    want,
                    &mut read,
                    null_mut(),
                )
            };
            if ok == 0 || read == 0 {
                return Err(format!("read failed ({})", unsafe { GetLastError() }));
            }
            filled += read as usize;
            offset += u64::from(read);
        }
        Ok(())
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn utf16_buf(buf: &[u16]) -> String {
        let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }
}
