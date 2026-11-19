use std::path::PathBuf;

/// Windows FILETIME epoch (1601-01-01) to Unix seconds.
pub fn filetime_to_unix(filetime: u64) -> u64 {
    const TICKS_PER_SEC: u64 = 10_000_000;
    const EPOCH_DIFF_SECS: u64 = 11_644_473_600;
    filetime
        .saturating_div(TICKS_PER_SEC)
        .saturating_sub(EPOCH_DIFF_SECS)
}

/// One file discovered from a walk or from the MFT, before it is linked into an index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawFile {
    pub file_ref: u64,
    pub parent_ref: u64,
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub flags: u16,
}

/// A volume the index knows about.
#[derive(Clone, Debug)]
pub struct Volume {
    pub mount: String,
    pub label: String,
    pub root: u32,
    pub journal_id: u64,
    pub next_usn: i64,
    pub case_insensitive: bool,
    pub method: u8,
}

/// A row the file pane can render. `size` for a directory is the rolled-up size when it came from the index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRow {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub hidden: bool,
    pub size: u64,
    pub modified: u64,
}

/// Separator used when an environment variable lists several index roots.
pub const FILE_ROW_SEP: char = '|';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_epoch_and_a_known_instant() {
        assert_eq!(filetime_to_unix(0), 0);
        // 2020-01-01 00:00:00 UTC.
        let unix = 1_577_836_800u64;
        let ticks = (unix + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime_to_unix(ticks), unix);
    }

    #[test]
    fn formats_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
    }
}
