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
    list_directory, squarify, sunburst, Clipboard, FileRow, Index, IndexBuilder, LiveChild,
    UnitRect, Weighted, METHOD_MFT, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE,
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
        name: String,
    },
    NewFile {
        dest: PathBuf,
        name: String,
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
        /// One count per row: alive entries in that subtree, including the row.
        item_counts: Vec<u64>,
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
    ShellImage {
        path: PathBuf,
        px: u32,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Error(String),
}

pub struct IconJob {
    pub path: PathBuf,
    pub px: u32,
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
) -> Sender<IconJob> {
    let (icon_tx, icon_rx) = std::sync::mpsc::channel();
    let icon_events = events.clone();
    let icon_repaint = repaint.clone();
    std::thread::Builder::new()
        .name("rustfiles-icons".into())
        .spawn(move || icon_loop(icon_rx, icon_events, icon_repaint))
        .expect("icon thread");
    std::thread::Builder::new()
        .name("rustfiles-index".into())
        .spawn(move || worker_loop(commands, events, repaint, progress))
        .expect("index thread");
    icon_tx
}

fn icon_loop(jobs: Receiver<IconJob>, events: Sender<Event>, repaint: egui::Context) {
    platform::init_com();
    while let Ok(job) = jobs.recv() {
        let image = platform::shell_image(&job.path, job.px);
        let (width, height, rgba) = image.unwrap_or((0, 0, Vec::new()));
        let _ = events.send(Event::ShellImage {
            path: job.path,
            px: job.px,
            width,
            height,
            rgba,
        });
        repaint.request_repaint();
    }
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

    let mut scan_job: Option<std::thread::JoinHandle<Index>> = None;
    loop {
        let command = commands.recv_timeout(Duration::from_millis(50));
        match command {
            Ok(command) => {
                let start_scan = matches!(command, Command::Boot | Command::Rescan);
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
                if start_scan {
                    // A full volume walk must not sit in front of directory listings.
                    // List commands keep being read on this thread while the walk runs.
                    let volumes_for_scan = volumes.clone();
                    let progress_for_scan = Arc::clone(&progress);
                    scan_job = Some(std::thread::spawn(move || {
                        lower_priority();
                        scan_volumes(&volumes_for_scan, &progress_for_scan)
                    }));
                }
                repaint.request_repaint();
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(job) = scan_job.as_ref() {
            if job.is_finished() {
                let built = scan_job
                    .take()
                    .expect("scan job")
                    .join()
                    .unwrap_or_else(|_| IndexBuilder::new().finish());
                let files = built.entry_count() as u64;
                save_index(Some(&built));
                index = Some(built);
                arm_watches(&mut watcher, &volumes, &mut watch_failed, &progress);
                let _ = events.send(Event::IndexReady {
                    files,
                    elapsed_ms: 0,
                });
                repaint.request_repaint();
            }
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
        if watch_failed && scan_job.is_none() && Instant::now() >= next_rescan && index.is_some() {
            let volumes_for_scan = volumes.clone();
            let progress_for_scan = Arc::clone(&progress);
            scan_job = Some(std::thread::spawn(move || {
                lower_priority();
                scan_volumes(&volumes_for_scan, &progress_for_scan)
            }));
            next_rescan = Instant::now() + Duration::from_secs(180);
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
        Command::Boot => {
            boot(index, volumes, events, progress, watcher, watch_failed);
        }
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
            // The worker loop starts the walk on another thread so listings stay live.
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
        Command::NewFolder { dest, name } => match rustfiles_core::create_directory(&dest, &name) {
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
        Command::NewFile { dest, name } => match rustfiles_core::create_text_file(&dest, &name) {
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
    let mut needs_scan = true;
    if let Ok(loaded) = Index::load(&path) {
        set_status(progress, "Loaded the saved index");
        *index = Some(loaded);
        let _ = events.send(Event::IndexReady {
            files: index.as_ref().map(Index::entry_count).unwrap_or(0) as u64,
            elapsed_ms: started.elapsed().as_millis(),
        });
        if let Some(current) = index.as_mut() {
            if catch_up(current, progress) {
                save_index(index.as_ref());
                needs_scan = false;
            }
        }
    }
    if needs_scan {
        set_status(progress, "indexing");
    } else {
        arm_watches(watcher, volumes, watch_failed, progress);
    }
    // `needs_scan` is observed by the caller via the Boot command, which always
    // starts the background walk. When catch-up succeeded the walk still refreshes
    // volumes that were not read from the USN journal. Listings do not wait for it.
    let _ = needs_scan;
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
        set_status(progress, &format!("indexing\t{}", volume.mount));
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
                        set_status(progress, "");
                    }
                    scan.files
                }
                Err(err) => {
                    // Access denied is the normal non-elevated path. Walk quietly.
                    if err != "access denied" {
                        set_status(progress, &format!("indexing\t{}", volume.mount));
                    }
                    builder.set_journal(id, 0, 0, rustfiles_core::METHOD_WALK);
                    Vec::new()
                }
            }
        } else {
            builder.set_journal(id, 0, 0, rustfiles_core::METHOD_WALK);
            Vec::new()
        };
        if files.is_empty() {
            rustfiles_core::walk_tree_into(Path::new(&volume.mount), &progress.files, |file| {
                builder.add_file(id, file);
            });
        } else {
            for file in files {
                builder.add_file(id, file);
            }
        }
    }
    let index = builder.finish();
    set_status(progress, &format!("ready\t{}", index.entry_count()));
    index
}

fn list(
    index: &mut Option<Index>,
    events: &Sender<Event>,
    path: PathBuf,
    show_hidden: bool,
    req: u64,
) {
    // The window always needs the real directory. An index node can exist and
    // still have no children while a volume walk is running, and returning that
    // empty node is what made every folder look empty.
    let disk = match list_directory(&path) {
        Ok(mut rows) => {
            if !show_hidden {
                rows.retain(|row| !row.hidden);
            }
            rows
        }
        Err(err) => {
            let _ = events.send(Event::Error(format!(
                "Cannot open {}: {err}",
                path.display()
            )));
            return;
        }
    };
    if let Some(index) = index.as_mut() {
        if let Some(id) = index.find_path(&path.display().to_string()) {
            let mut rows = index.list_dir(id, show_hidden);
            rows.retain(|row| show_hidden || !row.hidden);
            if !rows.is_empty() || disk.is_empty() {
                let _ = events.send(Event::Listing {
                    req,
                    path,
                    rows,
                    from_index: true,
                });
                return;
            }
        }
    }
    let _ = events.send(Event::Listing {
        req,
        path,
        rows: disk,
        from_index: false,
    });
}

fn search(index: &mut Option<Index>, events: &Sender<Event>, query: String, req: u64) {
    let Some(index) = index.as_mut() else {
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
    let item_counts: Vec<u64> = rows
        .iter()
        .map(|row| {
            index
                .find_path(&row.path.display().to_string())
                .map(|child| index.subtree_items(child))
                .unwrap_or(1)
        })
        .collect();
    let _ = req;
    let _ = events.send(Event::AnalyzeResult {
        path,
        total,
        rows,
        cells,
        item_counts,
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
    let image = image.thumbnail(160, 160);
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
    let mut errors = Vec::new();
    for path in paths {
        match op(path) {
            Ok(text) => {
                message = text;
                refresh = path.parent().map(Path::to_path_buf);
            }
            Err(err) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                errors.push(format!("{name}: {}", err.message));
            }
        }
    }
    if !errors.is_empty() {
        let _ = events.send(Event::Error(format!(
            "Delete failed: {}",
            errors.join("; ")
        )));
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

fn lower_priority() {
    rustfiles_core::set_walk_thread_hook(lower_priority_thread);
    lower_priority_thread();
}

fn lower_priority_thread() {
    #[cfg(target_os = "linux")]
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as u32;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

fn set_status(progress: &Progress, text: &str) {
    if let Ok(mut status) = progress.status.lock() {
        *status = text.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::{list, Event, Index};
    use rustfiles_core::FileRow;
    use rustfiles_core::IndexBuilder;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::mpsc;

    fn listing_rows(path: PathBuf, index: &mut Option<Index>) -> Vec<FileRow> {
        let (tx, rx) = mpsc::channel();
        list(index, &tx, path, false, 1);
        match rx.recv().expect("listing event") {
            Event::Listing { rows, .. } => rows,
            Event::Error(message) => panic!("listing failed: {message}"),
            _ => panic!("unexpected event"),
        }
    }

    #[test]
    fn listing_returns_directory_children_without_an_index() {
        let dir = std::env::temp_dir().join(format!("rustfiles-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("hello.txt"), b"hello").unwrap();
        let mut index = None;
        let rows = listing_rows(dir.clone(), &mut index);
        assert!(
            rows.iter().any(|row| row.name == "hello.txt"),
            "expected hello.txt, got {:?}",
            rows.iter().map(|row| &row.name).collect::<Vec<_>>()
        );
        assert!(rows.iter().any(|row| row.name == "sub" && row.is_dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_does_not_trust_an_empty_index_node() {
        let dir =
            std::env::temp_dir().join(format!("rustfiles-empty-index-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("present.txt"), b"x").unwrap();
        let mut builder = IndexBuilder::new();
        builder.add_volume(dir.display().to_string(), "temp".to_string(), cfg!(windows));
        let mut index = Some(builder.finish());
        let rows = listing_rows(dir.clone(), &mut index);
        assert!(
            rows.iter().any(|row| row.name == "present.txt"),
            "empty index node hid the real directory: {:?}",
            rows.iter().map(|row| &row.name).collect::<Vec<_>>()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Same `list` path the window uses. Windows CI must see a real system directory.
    #[cfg(windows)]
    #[test]
    fn windows_directory_lists_through_the_ui_command() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let path = PathBuf::from(&root);
        let mut index = None;
        let rows = listing_rows(path, &mut index);
        assert!(
            !rows.is_empty(),
            "{root} listed no files through the UI command"
        );
        assert!(
            rows.iter()
                .any(|row| row.name.eq_ignore_ascii_case("System32")),
            "System32 missing from {root}: {:?}",
            rows.iter()
                .map(|row| &row.name)
                .take(12)
                .collect::<Vec<_>>()
        );
    }
}
