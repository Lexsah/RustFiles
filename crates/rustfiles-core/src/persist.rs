//! `RFIDX001` snapshot. Derived structures are rebuilt on load.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::error::{Error, Result};
use crate::flags::TOMBSTONE;
use crate::index::{Index, NO_PARENT};
use crate::model::Volume;

const MAGIC: &[u8; 8] = b"RFIDX001";

impl Index {
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let payload = self.snapshot();
        let crc = crc32fast::hash(&payload);
        let tmp = path.with_extension("rfi.tmp");
        {
            let mut file = File::create(&tmp)?;
            file.write_all(MAGIC)?;
            file.write_all(&crc.to_le_bytes())?;
            file.write_all(&payload)?;
        }
        std::fs::rename(&tmp, path).or_else(|_| {
            std::fs::copy(&tmp, path)?;
            std::fs::remove_file(&tmp)?;
            Ok::<(), std::io::Error>(())
        })?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if bytes.len() < 12 || &bytes[0..8] != MAGIC {
            return Err(Error::new("Not a RustFiles index"));
        }
        let stored = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let payload = &bytes[12..];
        if crc32fast::hash(payload) != stored {
            return Err(Error::new("Index checksum mismatch"));
        }
        Self::from_snapshot(payload)
    }

    fn snapshot(&self) -> Vec<u8> {
        let mut map = vec![NO_PARENT; self.entries.len()];
        let mut live = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.flags & TOMBSTONE != 0 {
                continue;
            }
            map[index] = live.len() as u32;
            live.push(index);
        }
        let mut out = Vec::new();
        write_u32(&mut out, 1);
        write_u16(&mut out, self.volumes.len() as u16);
        write_u16(&mut out, 0);
        write_u32(&mut out, live.len() as u32);
        let next = self
            .entries
            .iter()
            .map(|entry| entry.file_ref)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        write_u64(&mut out, next);
        for volume in &self.volumes {
            write_str(&mut out, &volume.mount);
            write_str(&mut out, &volume.label);
            let root = map.get(volume.root as usize).copied().unwrap_or(NO_PARENT);
            write_u32(&mut out, root);
            write_u64(&mut out, volume.journal_id);
            out.extend_from_slice(&volume.next_usn.to_le_bytes());
            out.push(u8::from(volume.case_insensitive));
            out.push(volume.method);
            write_u16(&mut out, 0);
        }
        for index in live {
            let entry = &self.entries[index];
            let parent = if entry.parent == NO_PARENT {
                NO_PARENT
            } else {
                map.get(entry.parent as usize).copied().unwrap_or(NO_PARENT)
            };
            write_u32(&mut out, parent);
            write_u16(&mut out, entry.volume);
            write_u16(&mut out, entry.flags & !TOMBSTONE);
            write_u64(&mut out, entry.file_ref);
            write_u64(&mut out, entry.size);
            write_u64(&mut out, entry.modified);
            write_str(&mut out, self.name_of(index as u32));
        }
        out
    }

    fn from_snapshot(mut payload: &[u8]) -> Result<Self> {
        let version = read_u32(&mut payload)?;
        if version != 1 {
            return Err(Error::new(format!("Unsupported index version {version}")));
        }
        let volume_count = read_u16(&mut payload)? as usize;
        let _reserved = read_u16(&mut payload)?;
        let entry_count = read_u32(&mut payload)? as usize;
        let next_file_ref = read_u64(&mut payload)?;
        let mut volumes = Vec::with_capacity(volume_count);
        for _ in 0..volume_count {
            volumes.push(Volume {
                mount: read_str(&mut payload)?,
                label: read_str(&mut payload)?,
                root: read_u32(&mut payload)?,
                journal_id: read_u64(&mut payload)?,
                next_usn: read_i64(&mut payload)?,
                case_insensitive: read_u8(&mut payload)? != 0,
                method: read_u8(&mut payload)?,
            });
            let _pad = read_u16(&mut payload)?;
        }
        let mut index = Index::empty();
        index.volumes = volumes;
        index.next_file_ref = next_file_ref;
        for _ in 0..entry_count {
            let parent = read_u32(&mut payload)?;
            let volume = read_u16(&mut payload)?;
            let flags = read_u16(&mut payload)?;
            let file_ref = read_u64(&mut payload)?;
            let size = read_u64(&mut payload)?;
            let modified = read_u64(&mut payload)?;
            let name = read_str(&mut payload)?;
            index.push_named(volume, parent, file_ref, flags, size, modified, &name);
        }
        index.rebuild_derived();
        Ok(index)
    }

    #[allow(clippy::too_many_arguments)]
    fn push_named(
        &mut self,
        volume: u16,
        parent: u32,
        file_ref: u64,
        flags: u16,
        size: u64,
        modified: u64,
        name: &str,
    ) {
        let (name_start, name_len) = push_bytes(&mut self.names, name);
        self.entries.push(EntryAccess::entry(
            parent, volume, flags, file_ref, name_start, name_len, size, modified,
        ));
    }
}

/// Helper so `persist` can construct entries without making the field struct public.
pub(crate) struct EntryAccess;

impl EntryAccess {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn entry(
        parent: u32,
        volume: u16,
        flags: u16,
        file_ref: u64,
        name_start: u32,
        name_len: u16,
        size: u64,
        modified: u64,
    ) -> super::index::Entry {
        super::index::Entry {
            parent,
            volume,
            flags,
            file_ref,
            name_start,
            name_len,
            size,
            modified,
        }
    }
}

fn push_bytes(buf: &mut Vec<u8>, text: &str) -> (u32, u16) {
    let start = buf.len() as u32;
    let bytes = text.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    buf.extend_from_slice(&bytes[..len]);
    (start, len as u16)
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn write_str(out: &mut Vec<u8>, value: &str) {
    write_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn read_exact<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    if input.len() < len {
        return Err(Error::new("Truncated index"));
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}
fn read_u8(input: &mut &[u8]) -> Result<u8> {
    Ok(read_exact(input, 1)?[0])
}
fn read_u16(input: &mut &[u8]) -> Result<u16> {
    let bytes = read_exact(input, 2)?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}
fn read_u32(input: &mut &[u8]) -> Result<u32> {
    let bytes = read_exact(input, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}
fn read_u64(input: &mut &[u8]) -> Result<u64> {
    let bytes = read_exact(input, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}
fn read_i64(input: &mut &[u8]) -> Result<i64> {
    Ok(read_u64(input)? as i64)
}
fn read_str(input: &mut &[u8]) -> Result<String> {
    let len = read_u32(input)? as usize;
    let bytes = read_exact(input, len)?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::DIRECTORY;
    use crate::index::IndexBuilder;
    use crate::model::RawFile;

    #[test]
    fn round_trip_keeps_search_and_journal_cursor() {
        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume("/data".into(), "Data".into(), false);
        builder.set_journal(volume, 99, 1234, crate::METHOD_MFT);
        builder.add_file(volume, raw(1, 1, "", DIRECTORY));
        builder.add_file(volume, raw(2, 1, "Photos", DIRECTORY));
        builder.add_file(volume, raw(3, 2, "Annual Report.pdf", 0));
        let index = builder.finish();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.rfi");
        index.save(&path).unwrap();
        let mut loaded = Index::load(&path).unwrap();
        assert!(loaded
            .search("report", 5)
            .iter()
            .any(|hit| hit.name.contains("Report")));
        assert_eq!(loaded.volumes()[0].journal_id, 99);
        assert_eq!(loaded.volumes()[0].next_usn, 1234);
        assert_eq!(loaded.volumes()[0].method, crate::METHOD_MFT);
        let mut corrupted = std::fs::read(&path).unwrap();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        std::fs::write(&path, corrupted).unwrap();
        assert!(Index::load(&path).is_err());
    }

    fn raw(file_ref: u64, parent: u64, name: &str, flags: u16) -> RawFile {
        RawFile {
            file_ref,
            parent_ref: parent,
            name: name.into(),
            size: 4,
            modified: 1,
            flags,
        }
    }
}
