//! USN change-journal record parser.
//!
//! Microsoft documents `USN_RECORD_V2` and the read-journal output layout
//! (an `i64` next-USN followed by records). The Windows reader passes that
//! buffer here unchanged.

use crate::model::filetime_to_unix;

pub const USN_REASON_DATA_OVERWRITE: u32 = 0x0000_0001;
pub const USN_REASON_DATA_EXTEND: u32 = 0x0000_0002;
pub const USN_REASON_DATA_TRUNCATION: u32 = 0x0000_0004;
pub const USN_REASON_FILE_CREATE: u32 = 0x0000_0100;
pub const USN_REASON_FILE_DELETE: u32 = 0x0000_0200;
pub const USN_REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
pub const USN_REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
pub const USN_REASON_CLOSE: u32 = 0x8000_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnRecord {
    pub file_ref: u64,
    pub parent_ref: u64,
    pub usn: i64,
    pub timestamp: u64,
    pub reason: u32,
    pub attributes: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsnError {
    Truncated,
}

impl std::fmt::Display for UsnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated USN journal page"),
        }
    }
}

/// Parse one `FSCTL_READ_USN_JOURNAL` output buffer.
pub fn parse_usn_page(buf: &[u8]) -> Result<(i64, Vec<UsnRecord>), UsnError> {
    if buf.len() < 8 {
        return Err(UsnError::Truncated);
    }
    let next = i64::from_le_bytes(buf[0..8].try_into().unwrap());
    let mut pos = 8usize;
    let mut records = Vec::new();
    while pos + 4 <= buf.len() {
        let rec_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        if rec_len == 0 {
            break;
        }
        if rec_len < 60 || pos + rec_len > buf.len() {
            return Err(UsnError::Truncated);
        }
        let slice = &buf[pos..pos + rec_len];
        let major = u16::from_le_bytes([slice[4], slice[5]]);
        if major == 2 {
            records.push(parse_v2(slice)?);
        }
        pos += rec_len;
    }
    Ok((next, records))
}

fn parse_v2(slice: &[u8]) -> Result<UsnRecord, UsnError> {
    if slice.len() < 60 {
        return Err(UsnError::Truncated);
    }
    let name_len = u16::from_le_bytes([slice[56], slice[57]]) as usize;
    let name_off = u16::from_le_bytes([slice[58], slice[59]]) as usize;
    if name_off.saturating_add(name_len) > slice.len() || !name_len.is_multiple_of(2) {
        return Err(UsnError::Truncated);
    }
    let mut units = Vec::with_capacity(name_len / 2);
    let (chunks, _) = slice[name_off..name_off + name_len].as_chunks::<2>();
    for chunk in chunks {
        units.push(u16::from_le_bytes(*chunk));
    }
    Ok(UsnRecord {
        file_ref: u64::from_le_bytes(slice[8..16].try_into().unwrap()),
        parent_ref: u64::from_le_bytes(slice[16..24].try_into().unwrap()),
        usn: i64::from_le_bytes(slice[24..32].try_into().unwrap()),
        timestamp: filetime_to_unix(u64::from_le_bytes(slice[32..40].try_into().unwrap())),
        reason: u32::from_le_bytes(slice[40..44].try_into().unwrap()),
        attributes: u32::from_le_bytes(slice[52..56].try_into().unwrap()),
        name: String::from_utf16_lossy(&units),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_v2_record_after_the_next_usn() {
        let mut buf = vec![0u8; 8];
        buf[0..8].copy_from_slice(&90i64.to_le_bytes());
        let record = sample_record(
            7,
            (2 << 48) | 40,
            (1 << 48) | 5,
            "notes.txt",
            USN_REASON_FILE_CREATE | USN_REASON_CLOSE,
        );
        buf.extend_from_slice(&record);
        let (next, records) = parse_usn_page(&buf).unwrap();
        assert_eq!(next, 90);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "notes.txt");
        assert_eq!(records[0].file_ref, (2 << 48) | 40);
        assert_eq!(records[0].parent_ref, (1 << 48) | 5);
        assert_eq!(records[0].usn, 7);
        assert_ne!(records[0].reason & USN_REASON_FILE_CREATE, 0);
    }

    pub(crate) fn sample_record(
        usn: i64,
        file_ref: u64,
        parent: u64,
        name: &str,
        reason: u32,
    ) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let name_bytes = units.len() * 2;
        let raw = 60 + name_bytes;
        let len = raw.div_ceil(8) * 8;
        let mut rec = vec![0u8; len];
        rec[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        rec[4] = 2; // major
        rec[8..16].copy_from_slice(&file_ref.to_le_bytes());
        rec[16..24].copy_from_slice(&parent.to_le_bytes());
        rec[24..32].copy_from_slice(&usn.to_le_bytes());
        let ticks = (1_700_000_000u64 + 11_644_473_600) * 10_000_000;
        rec[32..40].copy_from_slice(&ticks.to_le_bytes());
        rec[40..44].copy_from_slice(&reason.to_le_bytes());
        rec[52..56].copy_from_slice(&0x20u32.to_le_bytes()); // archive
        rec[56..58].copy_from_slice(&(name_bytes as u16).to_le_bytes());
        rec[58..60].copy_from_slice(&60u16.to_le_bytes());
        for (index, unit) in units.iter().enumerate() {
            let pos = 60 + index * 2;
            rec[pos..pos + 2].copy_from_slice(&unit.to_le_bytes());
        }
        rec
    }
}
