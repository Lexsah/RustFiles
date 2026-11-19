//! Parallel directory walk used when the MFT is unavailable.
//!
//! The walk does not follow symlinks and, on Unix, does not cross onto another
//! device. That keeps one volume's index from absorbing another mount.

use std::fs::{self, Metadata};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

use crate::flags::{self, DIRECTORY, HIDDEN};
use crate::model::{FileRow, RawFile};

const SKIP_AT_ROOT: &[&str] = &["proc", "sys", "dev", "run"];

pub fn walk_tree(root: &Path, progress: &AtomicU64) -> Vec<RawFile> {
    let modified = modified_of(root);
    let counter = AtomicU64::new(2);
    let acc = Mutex::new(vec![RawFile {
        file_ref: 1,
        parent_ref: 1,
        name: String::new(),
        size: 0,
        modified,
        flags: DIRECTORY,
    }]);
    progress.fetch_add(1, Ordering::Relaxed);
    let mut level = vec![(root.to_path_buf(), 1u64, true)];
    while !level.is_empty() {
        let next = Mutex::new(Vec::new());
        level.par_iter().for_each(|(path, parent, top)| {
            let entries = match fs::read_dir(path) {
                Ok(entries) => entries,
                Err(_) => return,
            };
            let parent_dev = device_id(path);
            let mut local = Vec::new();
            let mut children = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "." || name == ".." {
                    continue;
                }
                if *top && SKIP_AT_ROOT.contains(&name.as_str()) {
                    continue;
                }
                let meta = match fs::symlink_metadata(entry.path()) {
                    Ok(meta) => meta,
                    Err(_) => continue,
                };
                let symlink = meta.file_type().is_symlink();
                let is_dir = meta.is_dir() && !symlink;
                if is_dir && !same_device(parent_dev, &entry.path()) {
                    continue;
                }
                let id = counter.fetch_add(1, Ordering::Relaxed);
                let mut file_flags = flags_of(&name, &meta, is_dir);
                if symlink {
                    file_flags |= flags::REPARSE;
                }
                local.push(RawFile {
                    file_ref: id,
                    parent_ref: *parent,
                    name: name.clone(),
                    size: if is_dir { 0 } else { meta.len() },
                    modified: modified_unix(&meta),
                    flags: file_flags,
                });
                progress.fetch_add(1, Ordering::Relaxed);
                if is_dir && !is_reparse(&meta) {
                    children.push((entry.path(), id, false));
                }
            }
            acc.lock().expect("walk accumulator").append(&mut local);
            next.lock().expect("walk queue").append(&mut children);
        });
        level = next.into_inner().expect("walk queue");
    }
    acc.into_inner().expect("walk accumulator")
}

pub fn list_directory(path: &Path) -> crate::Result<Vec<FileRow>> {
    let mut rows = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let symlink = meta.file_type().is_symlink();
        let is_dir = meta.is_dir() && !symlink;
        let flags = flags_of(&name, &meta, is_dir);
        rows.push(FileRow {
            path: entry.path(),
            name,
            is_dir,
            hidden: flags & HIDDEN != 0,
            size: if is_dir { 0 } else { meta.len() },
            modified: modified_unix(&meta),
        });
    }
    Ok(rows)
}

fn flags_of(name: &str, meta: &Metadata, is_dir: bool) -> u16 {
    let mut flags = 0u16;
    if is_dir {
        flags |= DIRECTORY;
    }
    if name.starts_with('.') {
        flags |= HIDDEN;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let attrs = meta.file_attributes();
        flags |= crate::flags::map_win_attrs(attrs);
        if is_dir {
            flags |= DIRECTORY;
        }
    }
    let _ = meta;
    flags
}

fn is_reparse(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

fn modified_of(path: &Path) -> u64 {
    fs::metadata(path)
        .ok()
        .map(|meta| modified_unix(&meta))
        .unwrap_or(0)
}

fn modified_unix(meta: &Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(unix)]
fn device_id(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|meta| meta.dev())
}

#[cfg(not(unix))]
fn device_id(_path: &Path) -> Option<u64> {
    None
}

fn same_device(parent: Option<u64>, path: &Path) -> bool {
    match (parent, device_id(path)) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexBuilder;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn walk_links_children_and_skips_symlink_targets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        File::create(root.join("a.txt"))
            .unwrap()
            .write_all(b"hello")
            .unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        File::create(root.join("sub").join("b.txt"))
            .unwrap()
            .write_all(b"xyzxyz")
            .unwrap();
        fs::create_dir(root.join(".hidden")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("sub", root.join("link-sub")).unwrap();
        }

        let progress = AtomicU64::new(0);
        let files = walk_tree(root, &progress);
        assert!(progress.load(Ordering::Relaxed) >= 4);

        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume(root.display().to_string(), "test".to_string(), false);
        for file in files {
            builder.add_file(volume, file);
        }
        let mut index = builder.finish();
        let nested = root.join("sub").join("b.txt");
        let id = index
            .find_path(&nested.display().to_string())
            .expect("nested file");
        assert_eq!(index.name_of(id), "b.txt");
        let hits = index.search("b.txt", 10);
        assert!(hits.iter().any(|hit| hit.name == "b.txt"));
        let root_id = index.find_path(&root.display().to_string()).unwrap();
        index.ensure_sizes();
        assert!(index.folder_size(root_id) >= 11);
        assert_eq!(index.search("b.txt", 10).len(), 1);
        let listing = index.list_dir(root_id, false);
        assert!(listing.iter().all(|row| row.name != ".hidden"));
        let with_hidden = index.list_dir(root_id, true);
        assert!(with_hidden.iter().any(|row| row.name == ".hidden"));
    }
}
