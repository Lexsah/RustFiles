//! Background owner of the filename index.
//!
//! The UI thread only sends commands and draws the latest rows. Directory
//! reads, search, and MFT/USN work happen here.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event as NotifyEvent, RecommendedWatcher, RecursiveMode, Watcher};
use rustfiles_core::flags::{DIRECTORY, HIDDEN};
use rustfiles_core::{
    list_directory, squarify, sunburst, walk_tree, Clipboard, FileRow, Index, IndexBuilder,
    LiveChild, UnitRect, Weighted, METHOD_MFT, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE,
    USN_REASON_DATA_TRUNCATION, USN_REASON_FILE_CREATE, USN_REASON_RENAME_NEW_NAME,
};

use crate::platform::{self, DetectedVolume};

pub enum Command {
    Boot,
    List {
        path: PathBuf,
        show_hidden: bool,
        req: u64,
    },
    Search {
        query: String,
        req: u64,
    },
    Analyze {
        path: PathBuf,
        req: u64,
    },
    Rescan,
    Paste {
        dest: PathBuf,
        clip: Clipboard,
    },
    Rename {
        path: PathBuf,
        new_name: String,
    },
    Trash {
        paths: Vec<PathBuf>,
    },
    DeletePermanent {
        paths: Vec<PathBuf>,
    },
    NewFolder {
        dest: PathBuf,
    },
    Preview {
        path: PathBuf,
    },
    /// Decode a small image thumbnail off the UI thread.
    Thumb {
        path: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub struct MapCell {
    pub name: String,
    pub size: u64,
    pub path: PathBuf,
    pub is_dir: bool,
    pub rect: UnitRect,
    pub start: f32,
    pub end: f32,
}

pub enum Event {
    Volumes(Vec<DetectedVolume>),
    IndexReady {
        files: u64,
        elapsed_ms: u128,
    },
    Listing {
        req: u64,
        path: PathBuf,
        rows: Vec<FileRow>,
        from_index: bool,
    },
    SearchResult {
        req: u64,
        query: String,
        rows: Vec<FileRow>,
    },
    AnalyzeResult {
        path: PathBuf,
        total: u64,
        rows: Vec<FileRow>,
        cells: Vec<MapCell>,
    },
    PreviewText {
        path: PathBuf,
        text: String,
    },
    PreviewImage {
        path: PathBuf,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    PreviewInfo {
        path: PathBuf,
        message: String,
    },
    OpDone {
        message: String,
        refresh: PathBuf,
    },
    Thumb {
        path: PathBuf,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Error(String),
}

pub struct Progress {
    pub files: AtomicU64,
    pub status: Mutex<String>,
}

pub fn spawn(
    commands: Receiver<Command>,
    events: Sender<Event>,
    repaint: egui::Context,
    progress: Arc<Progress>,
) {
    std::thread::Builder::new()
        .name("rustfiles-index".into())
        .spawn(move || worker_loop(commands, events, repaint, progress))
        .expect("index thread");
}

fn worker_loop(
    commands: Receiver<Command>,
    events: Sender<Event>,
    repaint: egui::Context,
    progress: Arc<Progress>,
) {
    let mut index: Option<Index> = None;
    let mut volumes = Vec::new();
    let (watch_tx, watch_rx) = std::sync::mpsc::channel();
    let mut watcher: Option<RecommendedWatcher> = notify::recommended_watcher(watch_tx).ok();
    let mut watch_failed = watcher.is_none();
    let mut pending_watch: Vec<PathBuf> = Vec::new();
    let mut last_watch = Instant::now();
    let mut next_rescan = Instant::now() + Duration::from_secs(180);

    loop {
        let command = commands.recv_timeout(Duration::from_millis(200));
        match command {
            Ok(command) => {
                if !handle(
                    command,
                    &mut index,
                    &mut volumes,
                    &events,
                    &progress,
                    &mut watcher,
                    &mut watch_failed,
                ) {
                    break;
                }
                repaint.request_repaint();
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        while let Ok(event) = watch_rx.try_recv() {
            if let Ok(event) = event {
                queue_watch(&mut pending_watch, &event);
                last_watch = Instant::now();
            }
        }
        if !pending_watch.is_empty() && last_watch.elapsed() > Duration::from_millis(400) {
            if let Some(index) = index.as_mut() {
                let paths = std::mem::take(&mut pending_watch);
                for path in paths {
                    sync_tree(index, &path, 0);
                }
            } else {
                pending_watch.clear();
            }
            repaint.request_repaint();
        }
        if watch_failed && Instant::now() >= next_rescan && index.is_some() {
            set_status(&progress, "Watch limit reached; rescanning volumes");
            index = Some(scan_volumes(&volumes, &progress));
            save_index(index.as_ref());
            let _ = events.send(Event::IndexReady {
                files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
                elapsed_ms: 0,
            });
            next_rescan = Instant::now() + Duration::from_secs(180);
            repaint.request_repaint();
        }
    }
}

fn handle(
    command: Command,
    index: &mut Option<Index>,
    volumes: &mut Vec<DetectedVolume>,
    events: &Sender<Event>,
    progress: &Progress,
    watcher: &mut Option<RecommendedWatcher>,
    watch_failed: &mut bool,
) -> bool {
    match command {
        Command::Boot => boot(index, volumes, events, progress, watcher, watch_failed),
        Command::List {
            path,
            show_hidden,
            req,
        } => list(index, events, path, show_hidden, req),
        Command::Search { query, req } => search(index, events, query, req),
        Command::Analyze { path, req } => analyze(index, events, path, req),
        Command::Rescan => {
            set_status(progress, "Rebuilding the filename index");
            progress.files.store(0, Ordering::Relaxed);
            *index = Some(scan_volumes(volumes, progress));
            save_index(index.as_ref());
            let _ = events.send(Event::IndexReady {
                files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
                elapsed_ms: 0,
            });
        }
        Command::Paste { dest, clip } => match rustfiles_core::paste(&clip, &dest) {
            Ok(paths) => {
                if let Some(index) = index.as_mut() {
                    sync_tree(index, &dest, 0);
                }
                let _ = events.send(Event::OpDone {
                    message: format!("Pasted {} item(s)", paths.len()),
                    refresh: dest,
                });
            }
            Err(err) => {
                let _ = events.send(Event::Error(err.message));
            }
        },
        Command::Rename { path, new_name } => match rustfiles_core::rename_path(&path, &new_name) {
            Ok(dest) => {
                if let Some(parent) = path.parent() {
                    if let Some(index) = index.as_mut() {
                        sync_tree(index, parent, 0);
                    }
                    let _ = events.send(Event::OpDone {
                        message: format!(
                            "Renamed to {}",
                            dest.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        refresh: parent.to_path_buf(),
                    });
                }
            }
            Err(err) => {
                let _ = events.send(Event::Error(err.message));
            }
        },
        Command::Trash { paths } => mutate_paths(index, events, &paths, |path| {
            rustfiles_core::delete_to_trash(path).map(|_| "Moved to trash".to_string())
        }),
        Command::DeletePermanent { paths } => mutate_paths(index, events, &paths, |path| {
            rustfiles_core::remove_permanently(path).map(|_| "Deleted".to_string())
        }),
        Command::NewFolder { dest } => match rustfiles_core::create_folder(&dest) {
            Ok(path) => {
                if let Some(index) = index.as_mut() {
                    sync_tree(index, &dest, 0);
                }
                let _ = events.send(Event::OpDone {
                    message: format!(
                        "Created {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    refresh: dest,
                });
            }
            Err(err) => {
                let _ = events.send(Event::Error(err.message));
            }
        },
        Command::Preview { path } => preview(events, &path),
        Command::Thumb { path } => {
            if let Ok(bytes) = fs::read(&path) {
                if let Some((width, height, rgba)) = decode_image(&bytes) {
                    let _ = events.send(Event::Thumb {
                        path,
                        width,
                        height,
                        rgba,
                    });
                }
            }
        }
    }
    true
}

fn boot(
    index: &mut Option<Index>,
    volumes: &mut Vec<DetectedVolume>,
    events: &Sender<Event>,
    progress: &Progress,
    watcher: &mut Option<RecommendedWatcher>,
    watch_failed: &mut bool,
) {
    *volumes = platform::detect_volumes();
    let _ = events.send(Event::Volumes(volumes.clone()));
    let started = Instant::now();
    let path = platform::index_path();
    if let Ok(loaded) = Index::load(&path) {
        set_status(progress, "Loaded the saved index");
        *index = Some(loaded);
        let _ = events.send(Event::IndexReady {
            files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
            elapsed_ms: started.elapsed().as_millis(),
        });
        if let Some(current) = index.as_mut() {
            if !catch_up(current, progress) {
                set_status(progress, "Journal cursor is stale; walking volumes");
                *index = Some(scan_volumes(volumes, progress));
                save_index(index.as_ref());
                let _ = events.send(Event::IndexReady {
                    files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
                    elapsed_ms: started.elapsed().as_millis(),
                });
            } else {
                save_index(index.as_ref());
            }
        }
    } else {
        set_status(progress, "Building the filename index");
        *index = Some(scan_volumes(volumes, progress));
        save_index(index.as_ref());
        let _ = events.send(Event::IndexReady {
            files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }
    arm_watches(watcher, volumes, watch_failed, progress);
}

fn catch_up(index: &mut Index, progress: &Progress) -> bool {
    let plan: Vec<(u16, String, u64, i64, u8)> = index
        .volumes()
        .iter()
        .enumerate()
        .map(|(id, volume)| {
            (
                id as u16,
                volume.mount.clone(),
                volume.journal_id,
                volume.next_usn,
                volume.method,
            )
        })
        .collect();
    if plan.is_empty() {
        return false;
    }
    for (id, mount, journal, next, method) in plan {
        if method != METHOD_MFT || journal == 0 {
            return false;
        }
        match platform::read_usn(&mount, journal, next) {
            Ok(catchup) => {
                set_status(
                    progress,
                    &format!("Applying {} USN records for {mount}", catchup.records.len()),
                );
                for record in &catchup.records {
                    index.apply_usn(id, record);
                    let interesting = record.reason
                        & (USN_REASON_FILE_CREATE
                            | USN_REASON_RENAME_NEW_NAME
                            | USN_REASON_DATA_EXTEND
                            | USN_REASON_DATA_TRUNCATION
                            | USN_REASON_DATA_OVERWRITE)
                        != 0;
                    if interesting {
                        if let Some(path) = index.path_for_ref(id, record.file_ref) {
                            if let Ok(meta) = fs::metadata(&path) {
                                let modified = meta
                                    .modified()
                                    .ok()
                                    .and_then(|time| {
                                        time.duration_since(std::time::UNIX_EPOCH).ok()
                                    })
                                    .map(|duration| duration.as_secs())
                                    .unwrap_or(0);
                                index.set_metadata(id, record.file_ref, meta.len(), modified);
                            }
                        }
                    }
                }
                index.set_journal(id, catchup.journal_id, catchup.next_usn);
            }
            Err(err) => {
                set_status(progress, &format!("USN catch-up skipped: {err}"));
                return false;
            }
        }
    }
    true
}

fn scan_volumes(volumes: &[DetectedVolume], progress: &Progress) -> Index {
    let mut builder = IndexBuilder::new();
    for volume in volumes {
        set_status(progress, &format!("Indexing {}", volume.mount));
        let id = builder.add_volume(
            volume.mount.clone(),
            volume.label.clone(),
            volume.case_insensitive,
        );
        let files = if volume.is_ntfs {
            match platform::scan_ntfs(&volume.mount, &progress.files) {
                Ok(scan) => {
                    builder.set_journal(id, scan.journal_id, scan.next_usn, METHOD_MFT);
                    if scan.journal_id == 0 {
                        set_status(
                            progress,
                            &format!("Read the MFT for {}. Run as administrator to follow the USN journal.", volume.mount),
                        );
                    }
                    scan.files
                }
                Err(err) => {
                    set_status(progress, &format!("{err}. Walking {}", volume.mount));
                    builder.set_journal(id, 0, 0, rustfiles_core::METHOD_WALK);
                    walk_tree(Path::new(&volume.mount), &progress.files)
                }
            }
        } else {
            builder.set_journal(id, 0, 0, rustfiles_core::METHOD_WALK);
            walk_tree(Path::new(&volume.mount), &progress.files)
        };
        for file in files {
            builder.add_file(id, file);
        }
    }
    let index = builder.finish();
    set_status(
        progress,
        &format!("Index ready, {} entries", index.entry_count()),
    );
    index
}

fn list(
    index: &mut Option<Index>,
    events: &Sender<Event>,
    path: PathBuf,
    show_hidden: bool,
    req: u64,
) {
    if let Some(index) = index.as_mut() {
        if let Some(id) = index.find_path(&path.display().to_string()) {
            let mut rows = index.list_dir(id, show_hidden);
            rows.retain(|row| show_hidden || !row.hidden);
            let _ = events.send(Event::Listing {
                req,
                path,
                rows,
                from_index: true,
            });
            return;
        }
    }
    match list_directory(&path) {
        Ok(mut rows) => {
            if !show_hidden {
                rows.retain(|row| !row.hidden);
            }
            let _ = events.send(Event::Listing {
                req,
                path,
                rows,
                from_index: false,
            });
        }
        Err(err) => {
            let _ = events.send(Event::Error(format!(
                "Cannot open {}: {err}",
                path.display()
            )));
        }
    }
}

fn search(index: &Option<Index>, events: &Sender<Event>, query: String, req: u64) {
    let Some(index) = index else {
        let _ = events.send(Event::SearchResult {
            req,
            query,
            rows: Vec::new(),
        });
        return;
    };
    let rows = index
        .search(&query, 400)
        .into_iter()
        .map(|hit| FileRow {
            name: hit.name,
            path: PathBuf::from(hit.path),
            is_dir: hit.is_dir,
            hidden: false,
            size: hit.size,
            modified: hit.modified,
        })
        .collect();
    let _ = events.send(Event::SearchResult { req, query, rows });
}

fn analyze(index: &mut Option<Index>, events: &Sender<Event>, path: PathBuf, req: u64) {
    let Some(index) = index.as_mut() else {
        let _ = events.send(Event::Error("The index is not ready yet".into()));
        return;
    };
    let Some(id) = index.find_path(&path.display().to_string()) else {
        let _ = events.send(Event::Error(format!(
            "{} is not in the index yet",
            path.display()
        )));
        return;
    };
    index.ensure_sizes();
    let total = index.folder_size(id);
    let mut rows = index.list_dir(id, true);
    rustfiles_core::sort_rows(&mut rows, rustfiles_core::SortColumn::Size, false, false);
    let weights: Vec<Weighted> = index.child_weights(id);
    let laid = squarify(
        &weights,
        UnitRect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        },
    );
    let arcs = sunburst(&weights);
    let cells: Vec<MapCell> = laid
        .into_iter()
        .enumerate()
        .map(|(index_cell, item)| {
            let row = rows.iter().find(|row| row.name == item.name);
            let arc = arcs.get(index_cell);
            MapCell {
                name: item.name,
                size: item.size,
                path: row.map(|row| row.path.clone()).unwrap_or_default(),
                is_dir: row.map(|row| row.is_dir).unwrap_or(false),
                rect: item.rect,
                start: arc.map(|arc| arc.start).unwrap_or(0.0),
                end: arc.map(|arc| arc.end).unwrap_or(0.0),
            }
        })
        .collect();
    let _ = req;
    let _ = events.send(Event::AnalyzeResult {
        path,
        total,
        rows,
        cells,
    });
}

fn preview(events: &Sender<Event>, path: &Path) {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            let _ = events.send(Event::PreviewInfo {
                path: path.to_path_buf(),
                message: err.to_string(),
            });
            return;
        }
    };
    if meta.is_dir() {
        let _ = events.send(Event::PreviewInfo {
            path: path.to_path_buf(),
            message: "Folder".into(),
        });
        return;
    }
    let ext = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp")
        && meta.len() <= 32 * 1024 * 1024
    {
        if let Ok(bytes) = fs::read(path) {
            if let Some((width, height, rgba)) = decode_image(&bytes) {
                let _ = events.send(Event::PreviewImage {
                    path: path.to_path_buf(),
                    width,
                    height,
                    rgba,
                });
                return;
            }
        }
    }
    match fs::File::open(path).and_then(|mut file| {
        use std::io::Read;
        let mut buf = vec![0u8; 64 * 1024];
        let read = file.read(&mut buf)?;
        buf.truncate(read);
        Ok(buf)
    }) {
        Ok(bytes) => {
            if bytes.iter().filter(|byte| **byte == 0).count() > bytes.len() / 20 {
                let _ = events.send(Event::PreviewInfo {
                    path: path.to_path_buf(),
                    message: format!("Binary file, {} bytes", meta.len()),
                });
            } else {
                let _ = events.send(Event::PreviewText {
                    path: path.to_path_buf(),
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                });
            }
        }
        Err(err) => {
            let _ = events.send(Event::PreviewInfo {
                path: path.to_path_buf(),
                message: err.to_string(),
            });
        }
    }
}

fn decode_image(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory(bytes).ok()?;
    let image = image.thumbnail(512, 512);
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((width, height, rgba.into_raw()))
}

fn mutate_paths(
    index: &mut Option<Index>,
    events: &Sender<Event>,
    paths: &[PathBuf],
    op: impl Fn(&Path) -> Result<String, rustfiles_core::Error>,
) {
    let mut refresh = None;
    let mut message = String::new();
    for path in paths {
        match op(path) {
            Ok(text) => {
                message = text;
                refresh = path.parent().map(Path::to_path_buf);
            }
            Err(err) => {
                let _ = events.send(Event::Error(err.message));
                return;
            }
        }
    }
    if let (Some(index), Some(dir)) = (index.as_mut(), refresh.clone()) {
        sync_tree(index, &dir, 0);
    }
    if let Some(refresh) = refresh {
        let _ = events.send(Event::OpDone { message, refresh });
    }
}

fn sync_tree(index: &mut Index, path: &Path, depth: u32) {
    if depth > 64 {
        return;
    }
    let Ok(rows) = list_directory(path) else {
        return;
    };
    let Some(id) = index.find_path(&path.display().to_string()) else {
        return;
    };
    let live: Vec<LiveChild> = rows
        .into_iter()
        .map(|row| LiveChild {
            flags: dir_flag(row.is_dir) | hidden_flag(row.hidden),
            name: row.name,
            is_dir: row.is_dir,
            size: row.size,
            modified: row.modified,
        })
        .collect();
    let created = index.reconcile_children(id, &live);
    for child in created {
        let child_path = PathBuf::from(index.path(child));
        sync_tree(index, &child_path, depth + 1);
    }
}

fn dir_flag(is_dir: bool) -> u16 {
    if is_dir {
        DIRECTORY
    } else {
        0
    }
}

fn hidden_flag(hidden: bool) -> u16 {
    if hidden {
        HIDDEN
    } else {
        0
    }
}

fn save_index(index: Option<&Index>) {
    if let Some(index) = index {
        let path = platform::index_path();
        if let Err(err) = index.save(&path) {
            eprintln!("failed to save index: {err}");
        }
    }
}

fn arm_watches(
    watcher: &mut Option<RecommendedWatcher>,
    volumes: &[DetectedVolume],
    watch_failed: &mut bool,
    progress: &Progress,
) {
    let Some(watcher) = watcher else {
        *watch_failed = true;
        return;
    };
    let mut failed = false;
    for volume in volumes {
        if watcher
            .watch(Path::new(&volume.mount), RecursiveMode::Recursive)
            .is_err()
        {
            failed = true;
        }
    }
    *watch_failed = failed;
    if failed {
        set_status(
            progress,
            "Filesystem watches did not cover every volume; periodic rescan is on",
        );
    }
}

fn queue_watch(pending: &mut Vec<PathBuf>, event: &NotifyEvent) {
    for path in &event.paths {
        let dir = if path.is_dir() {
            path.clone()
        } else {
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| path.clone())
        };
        if !pending.iter().any(|have| have == &dir) {
            pending.push(dir);
        }
    }
}

fn set_status(progress: &Progress, text: &str) {
    if let Ok(mut status) = progress.status.lock() {
        *status = text.to_string();
    }
}
