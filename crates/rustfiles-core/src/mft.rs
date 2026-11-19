//! Pure NTFS boot-sector, data-run, and `FILE` record parsers.
//!
//! The Windows binary reads the volume and hands the bytes here. Tests build
//! the same bytes on every platform.

use crate::flags::{self, DIRECTORY};
use crate::model::{filetime_to_unix, RawFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MftError {
    Truncated,
    BadBoot,
    BadRecord,
    Fixup,
    BadRun,
    TooLarge,
    Io(String),
}

impl std::fmt::Display for MftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated NTFS structure"),
            Self::BadBoot => write!(f, "not an NTFS boot sector"),
            Self::BadRecord => write!(f, "not a usable FILE record"),
            Self::Fixup => write!(f, "MFT fixup failed"),
            Self::BadRun => write!(f, "bad NTFS data run"),
            Self::TooLarge => write!(f, "MFT run is too large"),
            Self::Io(msg) => write!(f, "{msg}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootSector {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub cluster_size: u32,
    pub mft_lcn: u64,
    pub record_size: u32,
    pub volume_serial: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataRun {
    /// `None` is a sparse run.
    pub lcn: Option<u64>,
    pub clusters: u64,
}

pub fn parse_boot_sector(data: &[u8]) -> Result<BootSector, MftError> {
    if data.len() < 512 {
        return Err(MftError::Truncated);
    }
    if &data[3..11] != b"NTFS    " {
        return Err(MftError::BadBoot);
    }
    let bytes_per_sector = u16::from_le_bytes([data[0x0B], data[0x0C]]);
    let sectors_per_cluster = data[0x0D];
    if bytes_per_sector < 512
        || !bytes_per_sector.is_power_of_two()
        || sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
    {
        return Err(MftError::BadBoot);
    }
    let cluster_size = u32::from(bytes_per_sector) * u32::from(sectors_per_cluster);
    let mft_lcn = u64::from_le_bytes(data[0x30..0x38].try_into().unwrap());
    let record_size = mft_record_size(data[0x40] as i8, cluster_size);
    if record_size < 256 {
        return Err(MftError::BadBoot);
    }
    let volume_serial = u64::from_le_bytes(data[0x48..0x50].try_into().unwrap());
    Ok(BootSector {
        bytes_per_sector,
        sectors_per_cluster,
        cluster_size,
        mft_lcn,
        record_size,
        volume_serial,
    })
}

pub fn mft_record_size(clusters_per_record: i8, cluster_size: u32) -> u32 {
    if clusters_per_record > 0 {
        u32::from(clusters_per_record as u16).saturating_mul(cluster_size)
    } else if clusters_per_record < 0 {
        1u32 << (-i32::from(clusters_per_record) as u32)
    } else {
        1024
    }
}

pub fn apply_fixup(record: &mut [u8], bytes_per_sector: u16) -> Result<(), MftError> {
    if bytes_per_sector < 2 || record.len() < 8 {
        return Err(MftError::Truncated);
    }
    let usa_off = u16::from_le_bytes([record[4], record[5]]) as usize;
    let usa_count = u16::from_le_bytes([record[6], record[7]]) as usize;
    if usa_count == 0 || usa_off.saturating_add(usa_count.saturating_mul(2)) > record.len() {
        return Err(MftError::Fixup);
    }
    let usn = u16::from_le_bytes([record[usa_off], record[usa_off + 1]]);
    let sector = usize::from(bytes_per_sector);
    let sectors = record.len() / sector;
    if usa_count < sectors + 1 {
        return Err(MftError::Fixup);
    }
    for i in 0..sectors {
        let pos = (i + 1) * sector - 2;
        if pos + 2 > record.len() {
            return Err(MftError::Fixup);
        }
        let found = u16::from_le_bytes([record[pos], record[pos + 1]]);
        if found != usn {
            return Err(MftError::Fixup);
        }
        let src = usa_off + 2 + i * 2;
        record[pos] = record[src];
        record[pos + 1] = record[src + 1];
    }
    Ok(())
}

/// Data runs of the unnamed `$DATA` attribute. The record must already have its fixup applied.
pub fn unnamed_data_runs(record: &[u8]) -> Result<Vec<DataRun>, MftError> {
    if record.len() < 0x18 || record.len() < 4 || &record[..4] != b"FILE" {
        return Err(MftError::BadRecord);
    }
    let mut pos = u16::from_le_bytes([record[0x14], record[0x15]]) as usize;
    while pos + 8 <= record.len() {
        let attr_type = u32::from_le_bytes(record[pos..pos + 4].try_into().unwrap());
        if attr_type == 0xFFFF_FFFF {
            break;
        }
        let len = u32::from_le_bytes(record[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if len < 8 || pos + len > record.len() {
            break;
        }
        let nonresident = record[pos + 8] != 0;
        let name_len = record[pos + 9];
        if attr_type == 0x80 && nonresident && name_len == 0 && len >= 0x42 {
            let runs_off = u16::from_le_bytes([record[pos + 0x20], record[pos + 0x21]]) as usize;
            if runs_off < len {
                return parse_data_runs(&record[pos + runs_off..pos + len]);
            }
        }
        pos += len;
    }
    Err(MftError::BadRun)
}

pub fn parse_data_runs(data: &[u8]) -> Result<Vec<DataRun>, MftError> {
    let mut index = 0;
    let mut prev_lcn: i64 = 0;
    let mut runs = Vec::new();
    while index < data.len() {
        let header = data[index];
        index += 1;
        if header == 0 {
            break;
        }
        let len_len = (header & 0x0F) as usize;
        let off_len = (header >> 4) as usize;
        if len_len == 0 || index + len_len + off_len > data.len() {
            return Err(MftError::BadRun);
        }
        let clusters = read_uint(&data[index..index + len_len]);
        index += len_len;
        let lcn = if off_len == 0 {
            None
        } else {
            let delta = read_sint(&data[index..index + off_len]);
            index += off_len;
            prev_lcn = prev_lcn.wrapping_add(delta);
            if prev_lcn < 0 {
                return Err(MftError::BadRun);
            }
            Some(prev_lcn as u64)
        };
        runs.push(DataRun { lcn, clusters });
    }
    Ok(runs)
}

pub fn read_runs_to_vec<F>(
    cluster_size: u64,
    runs: &[DataRun],
    mut read_at: F,
) -> Result<Vec<u8>, MftError>
where
    F: FnMut(u64, &mut [u8]) -> Result<(), MftError>,
{
    if cluster_size == 0 {
        return Err(MftError::BadBoot);
    }
    let mut out = Vec::new();
    for run in runs {
        let bytes = run
            .clusters
            .checked_mul(cluster_size)
            .ok_or(MftError::TooLarge)?;
        if bytes > 8 * 1024 * 1024 * 1024 {
            return Err(MftError::TooLarge);
        }
        let start = out.len();
        out.resize(start + bytes as usize, 0);
        if let Some(lcn) = run.lcn {
            let offset = lcn.checked_mul(cluster_size).ok_or(MftError::TooLarge)?;
            read_at(offset, &mut out[start..])?;
        }
    }
    Ok(out)
}

struct ParsedName {
    parent_ref: u64,
    name: String,
    real_size: u64,
    modified: u64,
    flags: u32,
    namespace: u8,
}

struct ParsedRecord {
    in_use: bool,
    is_dir: bool,
    sequence: u16,
    names: Vec<ParsedName>,
    data_size: Option<u64>,
    modified: Option<u64>,
    win_attrs: u32,
}

/// Parse a logical `$MFT` stream. Record numbers start at zero.
pub fn parse_mft_stream(
    record_size: usize,
    bytes_per_sector: u16,
    stream: &[u8],
) -> Result<Vec<RawFile>, MftError> {
    parse_mft_stream_from(record_size, bytes_per_sector, stream, 0)
}

/// Like [`parse_mft_stream`], but the first record's number is `first_ordinal`.
/// Used to parse the MFT in chunks without holding the whole file.
pub fn parse_mft_stream_from(
    record_size: usize,
    bytes_per_sector: u16,
    stream: &[u8],
    first_ordinal: u64,
) -> Result<Vec<RawFile>, MftError> {
    if record_size < 256 || bytes_per_sector < 2 {
        return Err(MftError::BadRecord);
    }
    let mut out = Vec::new();
    let mut offset = 0;
    let mut ordinal = first_ordinal;
    while offset + record_size <= stream.len() {
        let slice = &stream[offset..offset + record_size];
        offset += record_size;
        let record_number = ordinal;
        ordinal += 1;
        if slice.iter().all(|byte| *byte == 0) || slice.len() < 4 || &slice[0..4] != b"FILE" {
            continue;
        }
        let Ok(parsed) = parse_file_record(slice, bytes_per_sector) else {
            continue;
        };
        if let Some(raw) = parsed.into_raw(record_number) {
            out.push(raw);
        }
    }
    Ok(out)
}

fn parse_file_record(record: &[u8], bytes_per_sector: u16) -> Result<ParsedRecord, MftError> {
    if record.len() < 0x30 || &record[0..4] != b"FILE" {
        return Err(MftError::BadRecord);
    }
    let mut owned = record.to_vec();
    apply_fixup(&mut owned, bytes_per_sector)?;
    let flags = u16::from_le_bytes([owned[0x16], owned[0x17]]);
    let in_use = flags & 0x01 != 0;
    let is_dir = flags & 0x02 != 0;
    let sequence = u16::from_le_bytes([owned[0x10], owned[0x11]]);
    let first = u16::from_le_bytes([owned[0x14], owned[0x15]]) as usize;
    let mut parsed = ParsedRecord {
        in_use,
        is_dir,
        sequence,
        names: Vec::new(),
        data_size: None,
        modified: None,
        win_attrs: 0,
    };
    if !in_use {
        return Ok(parsed);
    }
    let mut pos = first;
    while pos + 8 <= owned.len() {
        let attr_type = u32::from_le_bytes(owned[pos..pos + 4].try_into().unwrap());
        if attr_type == 0xFFFF_FFFF {
            break;
        }
        let len = u32::from_le_bytes(owned[pos + 4..pos + 8].try_into().unwrap()) as usize;
        if len < 8 || pos + len > owned.len() {
            break;
        }
        let nonresident = owned[pos + 8] != 0;
        let name_len = owned[pos + 9] as usize;
        if nonresident {
            if attr_type == 0x80 && name_len == 0 && len >= 0x40 {
                parsed.data_size = Some(u64::from_le_bytes(
                    owned[pos + 0x30..pos + 0x38].try_into().unwrap(),
                ));
            }
        } else if len >= 0x18 {
            let value_len =
                u32::from_le_bytes(owned[pos + 0x10..pos + 0x14].try_into().unwrap()) as usize;
            let value_off = u16::from_le_bytes([owned[pos + 0x14], owned[pos + 0x15]]) as usize;
            if pos + value_off + value_len <= pos + len {
                let value = &owned[pos + value_off..pos + value_off + value_len];
                match attr_type {
                    0x10 => parse_std_info(value, &mut parsed),
                    0x30 => {
                        if let Some(name) = parse_file_name(value) {
                            parsed.names.push(name);
                        }
                    }
                    0x80 if name_len == 0 => {
                        parsed.data_size = Some(value_len as u64);
                    }
                    _ => {}
                }
            }
        }
        pos += len;
    }
    Ok(parsed)
}

fn parse_std_info(value: &[u8], parsed: &mut ParsedRecord) {
    if value.len() >= 0x20 {
        parsed.modified = Some(filetime_to_unix(u64::from_le_bytes(
            value[8..16].try_into().unwrap(),
        )));
    }
    if value.len() >= 0x24 {
        parsed.win_attrs = u32::from_le_bytes(value[0x20..0x24].try_into().unwrap());
    }
}

fn parse_file_name(value: &[u8]) -> Option<ParsedName> {
    if value.len() < 0x42 {
        return None;
    }
    let chars = value[0x40] as usize;
    let byte_len = chars.saturating_mul(2);
    if value.len() < 0x42 + byte_len {
        return None;
    }
    let mut units = Vec::with_capacity(chars);
    let (chunks, _) = value[0x42..0x42 + byte_len].as_chunks::<2>();
    for chunk in chunks {
        units.push(u16::from_le_bytes(*chunk));
    }
    Some(ParsedName {
        parent_ref: u64::from_le_bytes(value[0..8].try_into().unwrap()),
        name: String::from_utf16_lossy(&units),
        real_size: u64::from_le_bytes(value[0x30..0x38].try_into().unwrap()),
        modified: filetime_to_unix(u64::from_le_bytes(value[0x10..0x18].try_into().unwrap())),
        flags: u32::from_le_bytes(value[0x38..0x3C].try_into().unwrap()),
        namespace: value[0x41],
    })
}

impl ParsedRecord {
    fn into_raw(self, record_number: u64) -> Option<RawFile> {
        if !self.in_use {
            return None;
        }
        let name = best_name(&self.names)?;
        let file_ref = (u64::from(self.sequence) << 48) | (record_number & 0x0000_FFFF_FFFF_FFFF);
        let mut flags = flags::map_win_attrs(self.win_attrs | name.flags);
        if self.is_dir || name.flags & flags::WIN_DIRECTORY != 0 {
            flags |= DIRECTORY;
        }
        let size = if flags & DIRECTORY != 0 {
            0
        } else {
            self.data_size.unwrap_or(name.real_size)
        };
        let modified = self.modified.unwrap_or(name.modified);
        Some(RawFile {
            file_ref,
            parent_ref: name.parent_ref,
            name: name.name.clone(),
            size,
            modified,
            flags,
        })
    }
}

fn best_name(names: &[ParsedName]) -> Option<&ParsedName> {
    names
        .iter()
        .find(|name| name.namespace == 1 || name.namespace == 3)
        .or_else(|| names.iter().find(|name| name.namespace != 2))
        .or_else(|| names.first())
}

fn read_uint(bytes: &[u8]) -> u64 {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().enumerate() {
        value |= u64::from(*byte) << (8 * index);
    }
    value
}

fn read_sint(bytes: &[u8]) -> i64 {
    let mut value = read_uint(bytes) as i64;
    let bits = bytes.len() * 8;
    if bits < 64 && bits > 0 && bytes[bytes.len() - 1] & 0x80 != 0 {
        value |= -1i64 << bits;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_sector_sizes() {
        let mut sector = vec![0u8; 512];
        sector[3..11].copy_from_slice(b"NTFS    ");
        sector[0x0B] = 0x00;
        sector[0x0C] = 0x02;
        sector[0x0D] = 8;
        sector[0x30] = 4;
        sector[0x40] = 0xF6; // -10 -> 1024-byte records
        sector[0x48] = 0x11;
        let boot = parse_boot_sector(&sector).unwrap();
        assert_eq!(boot.bytes_per_sector, 512);
        assert_eq!(boot.cluster_size, 4096);
        assert_eq!(boot.mft_lcn, 4);
        assert_eq!(boot.record_size, 1024);
        assert_eq!(boot.volume_serial, 0x11);
    }

    #[test]
    fn data_runs_sparse_and_negative_delta() {
        // 2 clusters at LCN 10, then a sparse 1, then 1 cluster at LCN 9 (delta -1 from 10).
        let bytes = [
            0x11, 0x02, 0x0A, // len=2, off=10
            0x01, 0x01, // sparse len=1
            0x11, 0x01, 0xFF, // len=1, delta=-1 -> LCN 9
            0x00,
        ];
        let runs = parse_data_runs(&bytes).unwrap();
        assert_eq!(
            runs,
            vec![
                DataRun {
                    lcn: Some(10),
                    clusters: 2
                },
                DataRun {
                    lcn: None,
                    clusters: 1
                },
                DataRun {
                    lcn: Some(9),
                    clusters: 1
                },
            ]
        );
    }

    #[test]
    fn fixup_restores_sector_tails() {
        let mut record = vec![0u8; 512];
        record[0..4].copy_from_slice(b"FILE");
        record[4] = 0x30;
        record[6] = 2;
        record[0x30] = 0x01;
        record[0x32] = 0xAB;
        record[0x33] = 0xCD;
        record[510] = 0x01;
        apply_fixup(&mut record, 512).unwrap();
        assert_eq!(&record[510..512], &[0xAB, 0xCD]);
    }

    #[test]
    fn fixup_rejects_a_mismatch() {
        let mut record = vec![0u8; 512];
        record[0..4].copy_from_slice(b"FILE");
        record[4] = 0x30;
        record[6] = 2;
        record[0x30] = 0x01;
        record[510] = 0x02;
        assert!(apply_fixup(&mut record, 512).is_err());
    }

    #[test]
    fn stream_yields_directory_and_file() {
        let root = file_record(RecordSpec {
            number: 5,
            sequence: 1,
            in_use: true,
            is_dir: true,
            parent: (1 << 48) | 5,
            name: ".",
            namespace: 1,
            file_flags: flags::WIN_DIRECTORY,
            data_size: None,
            modified_unix: 1_700_000_000,
        });
        let file_ref_parent = (1u64 << 48) | 5;
        let child = file_record(RecordSpec {
            number: 40,
            sequence: 2,
            in_use: true,
            is_dir: false,
            parent: file_ref_parent,
            name: "notes.txt",
            namespace: 1,
            file_flags: 0,
            data_size: Some(18),
            modified_unix: 1_700_000_100,
        });
        let mut stream = Vec::new();
        // Records 0..=4 empty, then root at 5. Pad to the child at 40.
        let record_size = 512;
        stream.resize(5 * record_size, 0);
        stream.extend_from_slice(&root);
        stream.resize(40 * record_size, 0);
        stream.extend_from_slice(&child);
        let files = parse_mft_stream(record_size, 512, &stream).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, ".");
        assert_eq!(files[0].file_ref, file_ref_parent);
        assert_eq!(files[0].parent_ref, file_ref_parent);
        assert_ne!(files[0].flags & DIRECTORY, 0);
        assert_eq!(files[1].name, "notes.txt");
        assert_eq!(files[1].size, 18);
        assert_eq!(files[1].parent_ref, file_ref_parent);
        assert_eq!(files[1].file_ref, (2u64 << 48) | 40);
        assert_eq!(files[1].modified, 1_700_000_100);
    }

    #[test]
    fn unnamed_data_attribute_exposes_its_runs() {
        let mut record = vec![0u8; 256];
        record[0..4].copy_from_slice(b"FILE");
        record[0x14] = 0x30;
        let at = 0x30;
        record[at..at + 4].copy_from_slice(&0x80u32.to_le_bytes());
        record[at + 4..at + 8].copy_from_slice(&0x48u32.to_le_bytes());
        record[at + 8] = 1;
        record[at + 0x20..at + 0x22].copy_from_slice(&0x40u16.to_le_bytes());
        record[at + 0x40] = 0x11;
        record[at + 0x41] = 0x01;
        record[at + 0x42] = 0x03;
        let runs = unnamed_data_runs(&record).unwrap();
        assert_eq!(
            runs,
            vec![DataRun {
                lcn: Some(3),
                clusters: 1
            }]
        );
    }

    #[test]
    fn runs_are_read_from_the_logical_cluster_offsets() {
        let cluster = 4u64;
        let mut disk = [0u8; 40];
        disk[8..16].copy_from_slice(b"abcdefgh");
        disk[28..32].copy_from_slice(b"WXYZ");
        let runs = vec![
            DataRun {
                lcn: Some(2),
                clusters: 2,
            },
            DataRun {
                lcn: None,
                clusters: 1,
            },
            DataRun {
                lcn: Some(7),
                clusters: 1,
            },
        ];
        let bytes = read_runs_to_vec(cluster, &runs, |offset, buf| {
            let start = offset as usize;
            buf.copy_from_slice(&disk[start..start + buf.len()]);
            Ok(())
        })
        .unwrap();
        assert_eq!(&bytes[0..8], b"abcdefgh");
        assert_eq!(&bytes[8..12], &[0, 0, 0, 0]);
        assert_eq!(&bytes[12..16], b"WXYZ");
    }

    struct RecordSpec {
        number: u32,
        sequence: u16,
        in_use: bool,
        is_dir: bool,
        parent: u64,
        name: &'static str,
        namespace: u8,
        file_flags: u32,
        data_size: Option<u64>,
        modified_unix: u64,
    }

    fn file_record(spec: RecordSpec) -> Vec<u8> {
        let mut record = vec![0u8; 512];
        record[0..4].copy_from_slice(b"FILE");
        record[4] = 0x30; // usa offset
        record[6] = 2; // usa count
        record[0x10..0x12].copy_from_slice(&spec.sequence.to_le_bytes());
        record[0x14] = 0x38; // first attribute
        let mut header_flags = if spec.in_use { 1u16 } else { 0 };
        if spec.is_dir {
            header_flags |= 2;
        }
        record[0x16..0x18].copy_from_slice(&header_flags.to_le_bytes());
        record[0x2C..0x30].copy_from_slice(&spec.number.to_le_bytes());
        let mut cursor = 0x38usize;
        cursor = write_std_info(&mut record, cursor, spec.modified_unix, spec.file_flags);
        cursor = write_file_name(
            &mut record,
            cursor,
            spec.parent,
            spec.name,
            spec.namespace,
            spec.file_flags,
            spec.modified_unix,
        );
        if let Some(size) = spec.data_size {
            let _ = write_data(&mut record, cursor, size);
        }
        // Fixup: USN 0x0001, saved tail 0x0000 already, stamp the sector end.
        record[0x30] = 0x01;
        record[0x31] = 0x00;
        record[510] = 0x01;
        record[511] = 0x00;
        let used = 400u32;
        record[0x18..0x1C].copy_from_slice(&used.to_le_bytes());
        record[0x1C..0x20].copy_from_slice(&512u32.to_le_bytes());
        record
    }

    fn write_std_info(record: &mut [u8], at: usize, modified_unix: u64, attrs: u32) -> usize {
        let value_len = 0x30usize;
        let total = align8(0x18 + value_len);
        record[at..at + 4].copy_from_slice(&0x10u32.to_le_bytes());
        record[at + 4..at + 8].copy_from_slice(&(total as u32).to_le_bytes());
        let value_off = 0x18u16;
        record[at + 0x10..at + 0x14].copy_from_slice(&(value_len as u32).to_le_bytes());
        record[at + 0x14..at + 0x16].copy_from_slice(&value_off.to_le_bytes());
        let ticks = (modified_unix + 11_644_473_600) * 10_000_000;
        let value = at + value_off as usize;
        record[value + 8..value + 16].copy_from_slice(&ticks.to_le_bytes());
        record[value + 0x20..value + 0x24].copy_from_slice(&attrs.to_le_bytes());
        at + total
    }

    fn write_file_name(
        record: &mut [u8],
        at: usize,
        parent: u64,
        name: &str,
        namespace: u8,
        attrs: u32,
        modified_unix: u64,
    ) -> usize {
        let units: Vec<u16> = name.encode_utf16().collect();
        let value_len = 0x42 + units.len() * 2;
        let total = align8(0x18 + value_len);
        record[at..at + 4].copy_from_slice(&0x30u32.to_le_bytes());
        record[at + 4..at + 8].copy_from_slice(&(total as u32).to_le_bytes());
        record[at + 0x10..at + 0x14].copy_from_slice(&(value_len as u32).to_le_bytes());
        record[at + 0x14..at + 0x16].copy_from_slice(&0x18u16.to_le_bytes());
        let value = at + 0x18;
        record[value..value + 8].copy_from_slice(&parent.to_le_bytes());
        let ticks = (modified_unix + 11_644_473_600) * 10_000_000;
        record[value + 0x10..value + 0x18].copy_from_slice(&ticks.to_le_bytes());
        record[value + 0x38..value + 0x3C].copy_from_slice(&attrs.to_le_bytes());
        record[value + 0x40] = units.len() as u8;
        record[value + 0x41] = namespace;
        for (index, unit) in units.iter().enumerate() {
            let pos = value + 0x42 + index * 2;
            record[pos..pos + 2].copy_from_slice(&unit.to_le_bytes());
        }
        at + total
    }

    fn write_data(record: &mut [u8], at: usize, size: u64) -> usize {
        let total = 0x40 + 8;
        let total = align8(total);
        record[at..at + 4].copy_from_slice(&0x80u32.to_le_bytes());
        record[at + 4..at + 8].copy_from_slice(&(total as u32).to_le_bytes());
        record[at + 8] = 1; // nonresident
        record[at + 0x20..at + 0x22].copy_from_slice(&0x40u16.to_le_bytes());
        record[at + 0x30..at + 0x38].copy_from_slice(&size.to_le_bytes());
        record[at + 0x40] = 0; // run terminator
        at + total
    }

    fn align8(value: usize) -> usize {
        value.div_ceil(8) * 8
    }
}
