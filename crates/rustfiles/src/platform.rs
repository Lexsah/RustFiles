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
    #[allow(dead_code)]
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

/// `(total_bytes, available_bytes)` for the volume that contains `path`.
pub fn disk_usage(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        let text = path.to_string_lossy();
        let c = std::ffi::CString::new(text.as_bytes()).ok()?;
        let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        let total = stat.f_blocks.saturating_mul(stat.f_frsize as u64);
        let free = stat.f_bavail.saturating_mul(stat.f_frsize as u64);
        Some((total, free))
    }
    #[cfg(windows)]
    {
        windows::disk_usage(path)
    }
}

/// System DPI as a scale factor (1.0 = 96 DPI). `None` off Windows.
pub fn window_scale() -> Option<f32> {
    #[cfg(windows)]
    {
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
        if dpi < 96 {
            None
        } else {
            Some(dpi as f32 / 96.0)
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Initialize COM on the calling thread so shell thumbnails can be created.
pub fn init_com() {
    #[cfg(windows)]
    {
        windows::init_com();
    }
}

/// Extend the window frame and ask Windows 11 for Mica Alt (the Files backdrop).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn enable_window_backdrop(hwnd: isize, dark: bool) {
    #[cfg(windows)]
    {
        windows::enable_window_backdrop(hwnd, dark);
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, dark);
    }
}

/// Shell icon or thumbnail. `edge` is the requested bitmap size in pixels.
pub fn shell_image(path: &Path, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(windows)]
    {
        windows::item_image(path, edge)
    }
    #[cfg(not(windows))]
    {
        let _ = edge;
        file_icon_rgba(path, path.is_dir())
    }
}

/// Shell bitmaps are premultiplied BGRA. egui wants straight RGBA.
///
/// A channel that is entirely zero is an icon with no alpha (treat it as opaque).
/// Otherwise a zero alpha is real transparency, even when the color bytes are white.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn straight_rgba_from_bgra(pixels: &mut [u8]) {
    let chunks = pixels.as_chunks_mut::<4>().0;
    let any_alpha = chunks.iter().any(|pixel| pixel[3] != 0);
    if !any_alpha {
        for pixel in chunks.iter_mut() {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        return;
    }
    for pixel in chunks.iter_mut() {
        let blue = u16::from(pixel[0]);
        let green = u16::from(pixel[1]);
        let red = u16::from(pixel[2]);
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = 0;
        } else if alpha == 255 {
            pixel[0] = red as u8;
            pixel[1] = green as u8;
            pixel[2] = blue as u8;
        } else {
            pixel[0] = ((red * 255) / alpha).min(255) as u8;
            pixel[1] = ((green * 255) / alpha).min(255) as u8;
            pixel[2] = ((blue * 255) / alpha).min(255) as u8;
        }
    }
}

/// Shell or mime type name, cached by extension.
pub fn describe_type(path: &Path, french: bool) -> String {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let key = format!("{}:{ext}", if french { "fr" } else { "en" });
    if let Ok(guard) = CACHE.lock() {
        if let Some(hit) = guard.as_ref().and_then(|map| map.get(&key)) {
            return hit.clone();
        }
    }
    let name = native_type_name(path, &ext, french);
    if let Ok(mut guard) = CACHE.lock() {
        guard
            .get_or_insert_with(std::collections::HashMap::new)
            .insert(key, name.clone());
    }
    name
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn native_type_name(path: &Path, ext: &str, french: bool) -> String {
    #[cfg(windows)]
    {
        if let Some(name) = windows::type_name(path) {
            if !name.is_empty() {
                return name;
            }
        }
    }
    fallback_type_name(ext, french)
}

fn fallback_type_name(ext: &str, french: bool) -> String {
    let (fr, en) = match ext {
        "" => ("Dossier", "Folder"),
        "png" => ("Fichier PNG", "PNG image"),
        "jpg" | "jpeg" => ("Fichier JPEG", "JPEG image"),
        "gif" | "bmp" | "webp" => ("Fichier image", "Image file"),
        "pdf" => ("Document PDF", "PDF document"),
        "exe" | "msi" => ("Application", "Application"),
        "zip" | "7z" | "rar" => ("Dossier compressé", "Compressed folder"),
        "json" => ("Fichier JSON", "JSON file"),
        "txt" | "md" | "log" => ("Document texte", "Text document"),
        "mp3" | "flac" | "wav" => ("Fichier audio", "Audio file"),
        "mp4" | "mkv" | "webm" => ("Fichier vidéo", "Video file"),
        "docx" | "doc" => ("Document Microsoft Word", "Microsoft Word document"),
        _ => ("", ""),
    };
    if !fr.is_empty() {
        return if french { fr.into() } else { en.into() };
    }
    let upper = ext.to_ascii_uppercase();
    if french {
        format!("Fichier {upper}")
    } else {
        format!("{upper} file")
    }
}

/// Small system icon for a path. Windows uses the shell; other platforms use the icon theme.
#[cfg_attr(windows, allow(dead_code))]
pub fn file_icon_rgba(path: &Path, is_dir: bool) -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(windows)]
    {
        windows::shell_icon(path, is_dir)
    }
    #[cfg(not(windows))]
    {
        freedesktop_icon(path, is_dir)
    }
}

#[cfg(not(windows))]
fn freedesktop_icon(path: &Path, is_dir: bool) -> Option<(u32, u32, Vec<u8>)> {
    let name = if is_dir {
        "folder"
    } else {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" => "image-x-generic",
            "mp3" | "flac" | "wav" | "ogg" => "audio-x-generic",
            "mp4" | "mkv" | "webm" => "video-x-generic",
            "txt" | "md" | "rs" | "json" | "toml" => "text-x-generic",
            "pdf" => "application-pdf",
            "zip" | "7z" | "tar" | "gz" => "package-x-generic",
            _ => "text-x-generic",
        }
    };
    for root in [
        "/usr/share/icons/Adwaita/16x16",
        "/usr/share/icons/Adwaita/22x22",
        "/usr/share/icons/Humanity/places/16",
        "/usr/share/icons/Humanity/mimes/16",
        "/usr/share/icons/hicolor/16x16",
    ] {
        for sub in ["places", "mimetypes", "apps", ""] {
            let candidate = if sub.is_empty() {
                format!("{root}/{name}.png")
            } else {
                format!("{root}/{sub}/{name}.png")
            };
            if let Ok(bytes) = std::fs::read(&candidate) {
                if let Some(image) = decode_png_icon(&bytes) {
                    return Some(image);
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn decode_png_icon(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((w, h, rgba.into_raw()))
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
            let code = unsafe { GetLastError() };
            // Error 5 is access denied. Callers walk the tree instead of reading the MFT.
            if code == 5 {
                return Err("access denied".into());
            }
            return Err(format!("Cannot open {path} ({code})"));
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

    pub fn disk_usage(path: &std::path::Path) -> Option<(u64, u64)> {
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide = wide(&path.display().to_string());
        let mut available = 0u64;
        let mut total = 0u64;
        let ok =
            unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, null_mut()) };
        if ok == 0 {
            None
        } else {
            Some((total, available))
        }
    }

    pub fn shell_icon(path: &std::path::Path, is_dir: bool) -> Option<(u32, u32, Vec<u8>)> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFOHEADER,
            BI_RGB, DIB_RGB_COLORS,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
        };
        use windows_sys::Win32::UI::Shell::{
            SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

        let wide = wide(&path.display().to_string());
        let mut info: SHFILEINFOW = unsafe { zeroed() };
        let attr = if is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };
        let flags = SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;
        let found = unsafe {
            SHGetFileInfoW(
                wide.as_ptr(),
                attr,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                flags,
            )
        };
        if found == 0 || info.hIcon.is_null() {
            return None;
        }
        let mut icon: ICONINFO = unsafe { zeroed() };
        if unsafe { GetIconInfo(info.hIcon, &mut icon) } == 0 {
            unsafe { DestroyIcon(info.hIcon) };
            return None;
        }
        let mut bitmap: BITMAP = unsafe { zeroed() };
        let object = unsafe {
            GetObjectW(
                icon.hbmColor,
                size_of::<BITMAP>() as i32,
                &mut bitmap as *mut BITMAP as *mut _,
            )
        };
        if object == 0 {
            unsafe {
                DeleteObject(icon.hbmColor);
                DeleteObject(icon.hbmMask);
                DestroyIcon(info.hIcon);
            }
            return None;
        }
        let width = bitmap.bmWidth.max(0) as u32;
        let height = bitmap.bmHeight.unsigned_abs();
        if width == 0 || height == 0 || width > 256 || height > 256 {
            unsafe {
                DeleteObject(icon.hbmColor);
                DeleteObject(icon.hbmMask);
                DestroyIcon(info.hIcon);
            }
            return None;
        }
        let mut header: BITMAPINFOHEADER = unsafe { zeroed() };
        header.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        header.biWidth = width as i32;
        header.biHeight = -(height as i32);
        header.biPlanes = 1;
        header.biBitCount = 32;
        header.biCompression = BI_RGB;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let rows = unsafe {
            GetDIBits(
                dc,
                icon.hbmColor,
                0,
                height,
                pixels.as_mut_ptr() as *mut _,
                &mut header as *mut BITMAPINFOHEADER as *mut _,
                DIB_RGB_COLORS,
            )
        };
        unsafe {
            ReleaseDC(std::ptr::null_mut(), dc);
            DeleteObject(icon.hbmColor);
            DeleteObject(icon.hbmMask);
            DestroyIcon(info.hIcon);
        }
        if rows == 0 {
            return None;
        }
        straight_rgba_from_bgra(&mut pixels);
        Some((width, height, pixels))
    }

    pub fn type_name(path: &std::path::Path) -> Option<String> {
        use std::mem::size_of;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
        use windows_sys::Win32::UI::Shell::{
            SHGetFileInfoW, SHFILEINFOW, SHGFI_TYPENAME, SHGFI_USEFILEATTRIBUTES,
        };
        let wide = wide(&path.display().to_string());
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        let found = unsafe {
            SHGetFileInfoW(
                wide.as_ptr(),
                FILE_ATTRIBUTE_NORMAL,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES,
            )
        };
        if found == 0 {
            return None;
        }
        let end = info
            .szTypeName
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(info.szTypeName.len());
        let name = String::from_utf16_lossy(&info.szTypeName[..end]);
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    pub fn init_com() {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let _ = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    }

    pub fn enable_window_backdrop(hwnd: isize, dark: bool) {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::Graphics::Dwm::{
            DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMNCRP_DISABLED,
            DWMSBT_MAINWINDOW, DWMWA_NCRENDERING_POLICY, DWMWA_SYSTEMBACKDROP_TYPE,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
        };
        use windows_sys::Win32::UI::Controls::MARGINS;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOMOVE,
            SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION,
        };
        let hwnd = hwnd as HWND;
        unsafe {
            let dark_mode = u32::from(dark);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &dark_mode as *const u32 as *const _,
                4,
            );
            // Stop DWM from painting its own caption buttons into the client area.
            let policy = DWMNCRP_DISABLED;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY as u32,
                &policy as *const i32 as *const _,
                4,
            );
            let style = GetWindowLongW(hwnd, GWL_STYLE);
            SetWindowLongW(hwnd, GWL_STYLE, style & !(WS_CAPTION as i32));
            let backdrop = DWMSBT_MAINWINDOW;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &backdrop as *const i32 as *const _,
                4,
            );
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let _ = SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
        }
    }

    pub fn item_image(path: &std::path::Path, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
        let edge = edge.clamp(16, 256);
        if let Some(image) = factory_image(path, edge) {
            return Some(image);
        }
        shell_icon(path, path.is_dir())
    }

    fn factory_image(path: &std::path::Path, edge: u32) -> Option<(u32, u32, Vec<u8>)> {
        use std::mem::size_of;
        use windows_sys::core::GUID;
        use windows_sys::Win32::Foundation::SIZE;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP};
        use windows_sys::Win32::UI::Shell::SHCreateItemFromParsingName;

        let wide = wide(&path.display().to_string());
        // IShellItemImageFactory
        let iid = GUID {
            data1: 0xbcc1_8b79,
            data2: 0xba16,
            data3: 0x442f,
            data4: [0x80, 0xc4, 0x8a, 0x59, 0xc3, 0x0c, 0x46, 0x3b],
        };
        let mut unknown: *mut std::ffi::c_void = std::ptr::null_mut();
        let hr = unsafe {
            SHCreateItemFromParsingName(wide.as_ptr(), std::ptr::null_mut(), &iid, &mut unknown)
        };
        if hr < 0 || unknown.is_null() {
            return None;
        }
        let vtbl = unsafe { *(unknown as *mut *mut usize) };
        type GetImage = unsafe extern "system" fn(
            *mut std::ffi::c_void,
            SIZE,
            i32,
            *mut *mut std::ffi::c_void,
        ) -> i32;
        type Release = unsafe extern "system" fn(*mut std::ffi::c_void) -> u32;
        let get_image: GetImage = unsafe { std::mem::transmute(*vtbl.add(3)) };
        let release: Release = unsafe { std::mem::transmute(*vtbl.add(2)) };
        let mut bitmap: *mut std::ffi::c_void = std::ptr::null_mut();
        // SIIGBF_BIGGERSIZEOK: thumbnail when the shell has one, otherwise the icon.
        let hr = unsafe {
            get_image(
                unknown,
                SIZE {
                    cx: edge as i32,
                    cy: edge as i32,
                },
                0x1,
                &mut bitmap,
            )
        };
        unsafe { release(unknown) };
        if hr < 0 || bitmap.is_null() {
            return None;
        }
        let mut info: BITMAP = unsafe { std::mem::zeroed() };
        let object = unsafe {
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                &mut info as *mut BITMAP as *mut _,
            )
        };
        if object == 0 {
            unsafe { DeleteObject(bitmap) };
            return None;
        }
        let image = hbitmap_rgba(bitmap, info.bmWidth, info.bmHeight.unsigned_abs());
        unsafe { DeleteObject(bitmap) };
        image
    }

    fn hbitmap_rgba(
        bitmap: *mut std::ffi::c_void,
        width: i32,
        height: u32,
    ) -> Option<(u32, u32, Vec<u8>)> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::Graphics::Gdi::{
            GetDC, GetDIBits, ReleaseDC, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        };
        let width = width.max(0) as u32;
        if width == 0 || height == 0 || width > 512 || height > 512 {
            return None;
        }
        let mut header: BITMAPINFOHEADER = unsafe { zeroed() };
        header.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        header.biWidth = width as i32;
        header.biHeight = -(height as i32);
        header.biPlanes = 1;
        header.biBitCount = 32;
        header.biCompression = BI_RGB;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let rows = unsafe {
            GetDIBits(
                dc,
                bitmap,
                0,
                height,
                pixels.as_mut_ptr() as *mut _,
                &mut header as *mut BITMAPINFOHEADER as *mut _,
                DIB_RGB_COLORS,
            )
        };
        unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        if rows == 0 {
            return None;
        }
        straight_rgba_from_bgra(&mut pixels);
        Some((width, height, pixels))
    }
}

#[cfg(test)]
mod pixel_tests {
    use super::straight_rgba_from_bgra;

    #[test]
    fn white_with_zero_alpha_stays_transparent_when_the_bitmap_has_alpha() {
        // Premultiplied BGRA: an opaque blue pixel, and a "transparent" pixel
        // stored as white with alpha 0. Promoting that white to opaque is the
        // square that covered folder labels.
        let mut pixels = vec![255, 0, 0, 255, 255, 255, 255, 0];
        straight_rgba_from_bgra(&mut pixels);
        assert_eq!(&pixels[0..4], &[0, 0, 255, 255]);
        assert_eq!(&pixels[4..8], &[0, 0, 0, 0]);
    }

    #[test]
    fn a_bitmap_with_no_alpha_channel_becomes_opaque() {
        let mut pixels = vec![10, 20, 30, 0, 1, 2, 3, 0];
        straight_rgba_from_bgra(&mut pixels);
        assert_eq!(&pixels[0..4], &[30, 20, 10, 255]);
        assert_eq!(&pixels[4..8], &[3, 2, 1, 255]);
    }

    #[test]
    fn premultiplied_color_is_unpremultiplied() {
        let mut pixels = vec![0, 0, 128, 128];
        straight_rgba_from_bgra(&mut pixels);
        assert_eq!(pixels, vec![255, 0, 0, 128]);
    }
}
