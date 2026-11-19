//! Parallel directory walk used when the MFT is unavailable.
//!
//! The walk does not follow symlinks and, on Unix, does not cross onto another
//! device. That keeps one volume's index from absorbing another mount.

use std::fs::{self, Metadata};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use std::sync::OnceLock;
use std::time::Duration;

use crate::flags::{self, DIRECTORY, HIDDEN};
use crate::model::{FileRow, RawFile};

const SKIP_AT_ROOT: &[&str] = &["proc", "sys", "dev", "run"];

fn walker_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .thread_name(|index| format!("rustfiles-walk-{index}"))
            .build()
            .expect("walk pool")
    })
}

static WALK_HOOK: std::sync::Mutex<Option<fn()>> = std::sync::Mutex::new(None);

/// Percent of one core the walk may use. 2 keeps an unelevated volume scan
/// under the 5% budget once UI repaints are also capped.
static WALK_DUTY_PERCENT: AtomicU32 = AtomicU32::new(2);

/// Sleep long enough that `work` is `percent` of (work + sleep).
pub fn pace_duration(work: Duration, percent: u32) -> Duration {
    let percent = percent.clamp(1, 100);
    if percent >= 100 || work.is_zero() {
        return Duration::ZERO;
    }
    work.saturating_mul(100 - percent) / percent
}

fn pace(work: Duration) {
    let sleep = pace_duration(work, WALK_DUTY_PERCENT.load(Ordering::Relaxed));
    if sleep >= Duration::from_millis(1) {
        std::thread::sleep(sleep);
    }
}

/// Called once on each walk thread. The binary uses this to drop thread priority.
pub fn set_walk_thread_hook(hook: fn()) {
    *WALK_HOOK.lock().expect("walk hook") = Some(hook);
}

fn call_walk_hook() {
    thread_local! {
        static ONCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    ONCE.with(|done| {
        if done.get() {
            return;
        }
        done.set(true);
        if let Some(hook) = *WALK_HOOK.lock().expect("walk hook") {
            hook();
        }
    });
}

pub fn walk_tree(root: &Path, progress: &AtomicU64) -> Vec<RawFile> {
    let mut files = Vec::new();
    walk_tree_into(root, progress, |file| files.push(file));
    files
}

/// Like [`walk_tree`], but hands each file to `sink` as soon as its directory
/// level is finished so a volume scan does not keep a second copy of every name.
pub fn walk_tree_into(root: &Path, progress: &AtomicU64, mut sink: impl FnMut(RawFile) + Send) {
    walker_pool().install(|| {
        call_walk_hook();
        walk_tree_inner(root, progress, &mut sink);
    });
}

fn walk_tree_inner(root: &Path, progress: &AtomicU64, sink: &mut (dyn FnMut(RawFile) + Send)) {
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
        let started = std::time::Instant::now();
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
        let mut batch = acc.lock().expect("walk accumulator");
        for file in batch.drain(..) {
            sink(file);
        }
        // Sleep from the measured cost of this level. A fixed pause per file
        // still used most of a core when reading NTFS metadata was the slow part.
        pace(started.elapsed());
    }
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

        WALK_DUTY_PERCENT.store(100, Ordering::Relaxed);
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

    #[test]
    fn throttled_walk_of_a_synthetic_tree() {
        let root = std::env::temp_dir().join(format!("rustfiles-bench-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let dirs: u32 = std::env::var("RUSTFILES_BENCH_DIRS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(80);
        let per_dir: u32 = std::env::var("RUSTFILES_BENCH_FILES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(100);
        for dir in 0..dirs {
            let folder = root.join(format!("dir-{dir}"));
            fs::create_dir(&folder).unwrap();
            for file in 0..per_dir {
                fs::write(folder.join(format!("f-{file}.bin")), vec![0u8; 32]).unwrap();
            }
        }
        WALK_DUTY_PERCENT.store(100, Ordering::Relaxed);
        let started = std::time::Instant::now();
        let cpu_before = process_cpu_ticks();
        let progress = AtomicU64::new(0);
        let files = walk_tree(&root, &progress);
        let elapsed = started.elapsed();
        let cpu_ticks = process_cpu_ticks().saturating_sub(cpu_before);
        let mut builder = IndexBuilder::new();
        let volume = builder.add_volume(root.display().to_string(), "bench".into(), false);
        for file in files {
            builder.add_file(volume, file);
        }
        let index = builder.finish();
        let count = index.entry_count().max(1);
        let bytes = index.resident_bytes();
        eprintln!(
            "synthetic walk files={count} threads={} elapsed_ms={} cpu_ms={} resident_bytes={bytes} bytes_per_entry={}",
            walker_pool().current_num_threads(),
            elapsed.as_millis(),
            cpu_ticks * 1000 / ticks_per_second(),
            bytes / count
        );
        assert!(count >= (dirs as usize) * (per_dir as usize));
        assert!(bytes / count < 100, "{}", bytes / count);
        let _ = fs::remove_dir_all(&root);
    }

    fn process_cpu_ticks() -> u64 {
        let Ok(text) = std::fs::read_to_string("/proc/self/stat") else {
            return 0;
        };
        let Some((_, rest)) = text.split_once(')') else {
            return 0;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let user = fields
            .get(11)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let system = fields
            .get(12)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        user + system
    }

    fn ticks_per_second() -> u64 {
        100
    }

    #[test]
    fn two_percent_duty_stays_under_five_percent_of_a_core() {
        let work = Duration::from_millis(10);
        let sleep = pace_duration(work, 2);
        assert_eq!(sleep, Duration::from_millis(490));
        let duty = work.as_secs_f64() / (work + sleep).as_secs_f64();
        assert!(duty < 0.05, "{duty}");
        assert_eq!(pace_duration(work, 100), Duration::ZERO);
    }
}
