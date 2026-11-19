//! Window state. Drawing lives in `ui.rs`; the index lives on the worker thread.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use egui::{Context, Key, Modifiers};
use rustfiles_core::{sort_rows, Clipboard, FileRow, SortColumn};
use serde::{Deserialize, Serialize};

use crate::nav::{self, Action};
use crate::platform::{self, DetectedVolume, Places};
use crate::worker::{self, Command, Event, MapCell, Progress};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Files,
    Analyzer,
}

#[derive(Clone)]
pub struct DeletePrompt {
    pub paths: Vec<PathBuf>,
    pub permanent: bool,
    pub bytes: u64,
    pub items: u64,
    pub label: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Details,
    List,
    Tiles,
    Grid,
    Large,
}

pub struct Tab {
    pub path: PathBuf,
    /// Home is the Files start page (drive cards and recent files), not a folder.
    pub home: bool,
    pub history: Vec<PathBuf>,
    pub hist_pos: usize,
    pub view: ViewMode,
    pub sort: SortColumn,
    pub ascending: bool,
    pub selected: Vec<PathBuf>,
    pub anchor: Option<usize>,
    pub req: u64,
    pub rows: Vec<FileRow>,
    pub from_index: bool,
    pub search: String,
    pub search_rows: Vec<FileRow>,
    pub search_active: bool,
}

impl Tab {
    pub fn at(path: PathBuf, home: bool) -> Self {
        Self {
            history: vec![path.clone()],
            hist_pos: 0,
            path,
            home,
            view: ViewMode::Tiles,
            sort: SortColumn::Name,
            ascending: true,
            selected: Vec::new(),
            anchor: None,
            req: 0,
            rows: Vec::new(),
            from_index: false,
            search: String::new(),
            search_rows: Vec::new(),
            search_active: false,
        }
    }
}

struct Smoke {
    path: PathBuf,
    saw_listing: bool,
    saw_search: bool,
    saw_analyze: bool,
}

#[derive(Serialize, Deserialize)]
struct Config {
    dark: bool,
    show_hidden: bool,
    show_preview: bool,
    favorites: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dark: true,
            show_hidden: false,
            show_preview: true,
            favorites: Vec::new(),
        }
    }
}

pub struct RustFilesApp {
    pub tabs: Vec<Tab>,
    pub current: usize,
    pub places: Places,
    pub volumes: Vec<DetectedVolume>,
    pub favorites: Vec<PathBuf>,
    pub dark: bool,
    pub show_hidden: bool,
    pub show_preview: bool,
    pub pane: Pane,
    pub analyzer_path: PathBuf,
    pub analyzer_total: u64,
    pub analyzer_rows: Vec<FileRow>,
    pub analyzer_cells: Vec<MapCell>,
    pub sunburst: bool,
    pub commands: Sender<Command>,
    events: Receiver<Event>,
    pub progress: Arc<Progress>,
    pub status: String,
    pub indexed: u64,
    pub indexing: bool,
    pub clipboard: Option<Clipboard>,
    pub path_edit: Option<String>,
    pub focus_path: bool,
    pub focus_search: bool,
    pub rename: Option<(PathBuf, String)>,
    pub confirm_delete: Vec<PathBuf>,
    pub delete_prompt: Option<DeletePrompt>,
    pub analyzer_selected: Option<PathBuf>,
    pub analyzer_counts: Vec<u64>,
    pub context_menu: Option<(egui::Pos2, PathBuf)>,
    pub help: bool,
    pub preview_path: PathBuf,
    pub preview_text: String,
    pub preview_image: Option<egui::TextureHandle>,
    pub preview_note: String,
    pub thumbs: HashMap<PathBuf, egui::TextureHandle>,
    pub icons_tex: HashMap<PathBuf, egui::TextureHandle>,
    pub thumb_asked: HashSet<PathBuf>,
    pub icon_asked: HashSet<(PathBuf, u32)>,
    pub icon_px: HashMap<PathBuf, u32>,
    thumb_order: VecDeque<PathBuf>,
    icon_order: VecDeque<PathBuf>,
    pub icon_tx: Sender<worker::IconJob>,
    pub mica_applied: bool,
    pub lang: crate::i18n::Lang,
    pub tr: crate::i18n::Strings,
    pub open_fav: bool,
    pub open_drives: bool,
    pub open_cloud: bool,
    pub open_network: bool,
    pub open_wsl: bool,
    pub open_tags: bool,
    pub new_menu: bool,
    pub layout_menu: bool,
    pub layout_log: Vec<(String, [f32; 4])>,
    pub shot_path: Option<PathBuf>,
    pub layout_path: Option<PathBuf>,
    pub shot_sent: bool,
    pub settings_menu: bool,
    pub cache: HashMap<PathBuf, Vec<FileRow>>,
    req: u64,
    smoke: Option<Smoke>,
    started: Instant,
}

impl RustFilesApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = load_config();
        install_fonts(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, config.dark);
        let (commands, command_rx) = std::sync::mpsc::channel();
        let (event_tx, events) = std::sync::mpsc::channel();
        let progress = Arc::new(Progress {
            files: std::sync::atomic::AtomicU64::new(0),
            status: std::sync::Mutex::new("Starting".into()),
        });
        let icon_tx = worker::spawn(
            command_rx,
            event_tx,
            cc.egui_ctx.clone(),
            Arc::clone(&progress),
        );
        let _ = commands.send(Command::Boot);
        let places = platform::places();
        let smoke_path = std::env::var("RUSTFILES_SMOKE").ok().map(PathBuf::from);
        let opened = std::env::var("RUSTFILES_OPEN").ok().map(PathBuf::from);
        let mut start = smoke_path
            .clone()
            .or_else(|| opened.clone())
            .unwrap_or_else(|| places.home.clone());
        let mut select_file = None;
        if smoke_path.is_none() && start.is_file() {
            select_file = Some(start.clone());
            if let Some(parent) = start.parent() {
                if !parent.as_os_str().is_empty() {
                    start = parent.to_path_buf();
                }
            }
        }
        let mut favorites = config.favorites;
        if favorites.is_empty() {
            favorites = vec![
                places.home.clone(),
                places.desktop.clone(),
                places.documents.clone(),
                places.downloads.clone(),
                places.pictures.clone(),
                places.music.clone(),
                places.videos.clone(),
            ];
        }
        let home = smoke_path.is_none() && opened.is_none();
        let lang = crate::i18n::system_lang();
        let tr = crate::i18n::strings(lang);
        let mut app = Self {
            tabs: vec![Tab::at(start.clone(), home)],
            current: 0,
            places,
            volumes: Vec::new(),
            favorites,
            dark: config.dark,
            show_hidden: config.show_hidden,
            show_preview: config.show_preview,
            pane: Pane::Files,
            analyzer_path: start,
            analyzer_total: 0,
            analyzer_rows: Vec::new(),
            analyzer_cells: Vec::new(),
            sunburst: false,
            commands,
            events,
            progress,
            status: "Starting".into(),
            indexed: 0,
            indexing: true,
            clipboard: None,
            path_edit: None,
            focus_path: false,
            focus_search: false,
            rename: None,
            confirm_delete: Vec::new(),
            delete_prompt: None,
            analyzer_selected: None,
            analyzer_counts: Vec::new(),
            context_menu: None,
            help: false,
            preview_path: PathBuf::new(),
            preview_text: String::new(),
            preview_image: None,
            preview_note: String::new(),
            thumbs: HashMap::new(),
            icons_tex: HashMap::new(),
            thumb_asked: HashSet::new(),
            icon_asked: HashSet::new(),
            icon_px: HashMap::new(),
            thumb_order: VecDeque::new(),
            icon_order: VecDeque::new(),
            icon_tx,
            mica_applied: false,
            lang,
            tr,
            open_fav: true,
            open_drives: true,
            open_cloud: false,
            open_network: false,
            open_wsl: false,
            open_tags: false,
            new_menu: false,
            layout_menu: false,
            layout_log: Vec::new(),
            shot_path: std::env::var("RUSTFILES_SHOT").ok().map(PathBuf::from),
            layout_path: std::env::var("RUSTFILES_LAYOUT").ok().map(PathBuf::from),
            shot_sent: false,
            settings_menu: false,
            cache: HashMap::new(),
            req: 1,
            smoke: smoke_path.map(|path| Smoke {
                path,
                saw_listing: false,
                saw_search: false,
                saw_analyze: false,
            }),
            started: Instant::now(),
        };
        if let Some(rows) = app.cache.get(&app.tabs[0].path).cloned() {
            app.tabs[0].rows = rows;
        }
        if !home {
            app.request_list();
        }
        if let Some(file) = select_file {
            app.tabs[0].selected = vec![file];
        }
        if let Ok(mode) = std::env::var("RUSTFILES_VIEW") {
            app.tabs[0].view = match mode.as_str() {
                "details" => ViewMode::Details,
                "list" => ViewMode::List,
                "grid" => ViewMode::Grid,
                "large" => ViewMode::Large,
                _ => ViewMode::Tiles,
            };
        }
        app
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.current]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.current]
    }

    pub fn pump(&mut self, ctx: &Context) {
        let mut uploads = 0;
        while let Ok(event) = self.events.try_recv() {
            let heavy = matches!(event, Event::ShellImage { .. } | Event::Thumb { .. });
            self.on_event(event, ctx);
            if heavy {
                uploads += 1;
                if uploads >= 4 {
                    ctx.request_repaint();
                    break;
                }
            }
        }
        if let Ok(status) = self.progress.status.try_lock() {
            if !status.is_empty() {
                self.status = status.clone();
            }
        }
    }

    fn on_event(&mut self, event: Event, ctx: &Context) {
        match event {
            Event::Volumes(volumes) => self.volumes = volumes,
            Event::IndexReady { files, elapsed_ms } => {
                self.indexed = files;
                self.indexing = false;
                self.status = format!("Index ready: {files} entries in {elapsed_ms} ms");
                self.request_list();
                if self.pane == Pane::Analyzer {
                    self.request_analyze(self.analyzer_path.clone());
                }
            }
            Event::Listing {
                req,
                path,
                rows,
                from_index,
            } => {
                if paths_match(&self.tab().path, &path) {
                    let mut rows = rows;
                    let sort = self.tab().sort;
                    let ascending = self.tab().ascending;
                    sort_rows(&mut rows, sort, ascending, true);
                    self.cache.insert(path, rows.clone());
                    let accept = self.tab().req == req || req == 0 || self.tab().rows.is_empty();
                    if accept {
                        let tab = self.tab_mut();
                        tab.rows = rows;
                        tab.from_index = from_index;
                    }
                }
            }
            Event::SearchResult { req, query, rows } => {
                let tab = self.tab_mut();
                if tab.search == query {
                    let _ = req;
                    tab.search_rows = rows;
                    tab.search_active = false;
                }
            }
            Event::AnalyzeResult {
                path,
                total,
                rows,
                cells,
                item_counts,
            } => {
                self.analyzer_path = path;
                self.analyzer_total = total;
                self.analyzer_rows = rows;
                self.analyzer_cells = cells;
                self.analyzer_counts = item_counts;
                self.pane = Pane::Analyzer;
            }
            Event::PreviewText { path, text } => {
                self.preview_path = path;
                self.preview_text = text;
                self.preview_note.clear();
                self.preview_image = None;
            }
            Event::PreviewImage {
                path,
                width,
                height,
                rgba,
            } => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba,
                );
                self.preview_image =
                    Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
                self.preview_path = path;
                self.preview_text.clear();
                self.preview_note.clear();
            }
            Event::PreviewInfo { path, message } => {
                self.preview_path = path;
                self.preview_note = message;
                self.preview_text.clear();
                self.preview_image = None;
            }
            Event::OpDone { message, refresh } => {
                self.status = message;
                if self.tab().path == refresh {
                    self.request_list();
                }
                if self.pane == Pane::Analyzer
                    && (self.analyzer_path == refresh || self.analyzer_path.starts_with(&refresh))
                {
                    self.analyzer_selected = None;
                    let path = self.analyzer_path.clone();
                    self.request_analyze(path);
                }
            }
            Event::Thumb {
                path,
                width,
                height,
                rgba,
            } => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba,
                );
                self.thumbs.insert(
                    path.clone(),
                    ctx.load_texture(
                        path.display().to_string(),
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                );
                self.thumb_order.retain(|have| have != &path);
                self.thumb_order.push_back(path);
                while self.thumb_order.len() > 256 {
                    if let Some(old) = self.thumb_order.pop_front() {
                        self.thumbs.remove(&old);
                        self.thumb_asked.remove(&old);
                    }
                }
            }
            Event::ShellImage {
                path,
                px,
                width,
                height,
                rgba,
            } => {
                if width == 0 || rgba.is_empty() {
                    self.icon_asked.insert((path, px));
                    return;
                }
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &rgba,
                );
                let have = self.icon_px.get(&path).copied().unwrap_or(0);
                if px >= have {
                    self.icons_tex.insert(
                        path.clone(),
                        ctx.load_texture(
                            format!("{}#{px}", path.display()),
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                    self.icon_px.insert(path.clone(), px);
                    self.icon_order.retain(|have| have != &path);
                    self.icon_order.push_back(path);
                    while self.icon_order.len() > 256 {
                        if let Some(old) = self.icon_order.pop_front() {
                            self.icons_tex.remove(&old);
                            self.icon_px.remove(&old);
                            self.icon_asked.retain(|(asked, _)| asked != &old);
                        }
                    }
                }
            }
            Event::Error(message) => {
                let scan_noise = message.contains("Cannot open \\\\.\\")
                    || (message.contains("access denied") && !message.starts_with("Delete failed"));
                if !scan_noise {
                    self.status = message;
                }
            }
        }
    }

    pub fn keys(&mut self, ctx: &Context) {
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            self.escape();
            return;
        }
        if ctx.text_edit_focused() {
            return;
        }
        let action = ctx.input_mut(|input| {
            let mods = input.modifiers;
            [
                Key::T,
                Key::W,
                Key::Tab,
                Key::L,
                Key::F,
                Key::A,
                Key::C,
                Key::X,
                Key::V,
                Key::N,
                Key::ArrowLeft,
                Key::ArrowRight,
                Key::P,
                Key::Delete,
                Key::F1,
                Key::F2,
                Key::F5,
                Key::Backspace,
                Key::Enter,
            ]
            .into_iter()
            .find_map(|key| {
                if input.key_pressed(key) {
                    nav::shortcut(mods, key)
                } else {
                    None
                }
            })
        });
        if let Some(action) = action {
            self.perform(action);
        }
        let down = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
        let up = ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
        if down {
            self.move_selection(1);
        } else if up {
            self.move_selection(-1);
        }
    }

    fn escape(&mut self) {
        self.path_edit = None;
        self.help = false;
        self.rename = None;
        self.confirm_delete.clear();
        self.delete_prompt = None;
        self.context_menu = None;
        self.layout_menu = false;
        self.new_menu = false;
        self.settings_menu = false;
        let tab = self.tab_mut();
        if !tab.search.is_empty() {
            tab.search.clear();
            tab.search_rows.clear();
        }
    }

    pub fn perform(&mut self, action: Action) {
        match action {
            Action::NewTab => self.new_tab(),
            Action::CloseTab => self.close_tab(),
            Action::NextTab => {
                if !self.tabs.is_empty() {
                    self.current = (self.current + 1) % self.tabs.len();
                }
            }
            Action::Back => self.history_step(-1),
            Action::Forward => self.history_step(1),
            Action::Up => {
                if let Some(parent) = nav::parent_dir(&self.tab().path) {
                    self.navigate(parent);
                }
            }
            Action::FocusPath => {
                self.path_edit = Some(self.tab().path.display().to_string());
                self.focus_path = true;
            }
            Action::FocusSearch => self.focus_search = true,
            Action::Refresh => {
                let path = self.tab().path.clone();
                self.cache.remove(&path);
                self.request_list();
            }
            Action::Rename => self.begin_rename(),
            Action::Trash => self.begin_delete(false),
            Action::DeletePermanent => self.begin_delete(true),
            Action::Copy => self.set_clipboard(false),
            Action::Cut => self.set_clipboard(true),
            Action::Paste => {
                if let Some(clip) = self.clipboard.clone() {
                    let dest = if self.pane == Pane::Analyzer {
                        self.analyzer_path.clone()
                    } else {
                        self.tab().path.clone()
                    };
                    let _ = self.commands.send(Command::Paste { dest, clip });
                } else {
                    self.status = self.tr.nothing_to_paste.into();
                }
            }
            Action::SelectAll => {
                let paths = self
                    .visible_rows()
                    .iter()
                    .map(|row| row.path.clone())
                    .collect();
                self.tab_mut().selected = paths;
            }
            Action::NewFolder => {
                let dest = self.tab().path.clone();
                let name = self.tr.new_folder_name.to_string();
                let _ = self.commands.send(Command::NewFolder { dest, name });
            }
            Action::TogglePreview => self.show_preview = !self.show_preview,
            Action::Analyzer => {
                let path = self.tab().path.clone();
                self.request_analyze(path);
            }
            Action::Help => self.help = !self.help,
            Action::Escape => self.escape(),
            Action::Open => self.open_selection(),
        }
    }

    pub fn navigate(&mut self, path: PathBuf) {
        self.pane = Pane::Files;
        {
            let tab = self.tab_mut();
            tab.home = false;
            if tab.path != path {
                tab.history.truncate(tab.hist_pos + 1);
                tab.history.push(path.clone());
                tab.hist_pos = tab.history.len() - 1;
                tab.path = path.clone();
                tab.selected.clear();
                tab.anchor = None;
            }
        }
        if let Some(cached) = self.cache.get(&path).cloned() {
            self.tab_mut().rows = cached;
        }
        self.request_list();
    }

    fn history_step(&mut self, delta: isize) {
        let tab = self.tab_mut();
        let next = tab.hist_pos as isize + delta;
        if next < 0 || next as usize >= tab.history.len() {
            return;
        }
        tab.hist_pos = next as usize;
        tab.path = tab.history[tab.hist_pos].clone();
        tab.home = false;
        tab.selected.clear();
        let path = tab.path.clone();
        if let Some(cached) = self.cache.get(&path).cloned() {
            self.tab_mut().rows = cached;
        }
        self.request_list();
    }

    pub fn request_list(&mut self) {
        self.req = self.req.wrapping_add(1);
        let req = self.req;
        let path = self.tab().path.clone();
        let show_hidden = self.show_hidden;
        self.tab_mut().req = req;
        let _ = self.commands.send(Command::List {
            path,
            show_hidden,
            req,
        });
    }

    pub fn request_search(&mut self) {
        let query = self.tab().search.clone();
        self.req = self.req.wrapping_add(1);
        let req = self.req;
        self.tab_mut().search_active = true;
        let _ = self.commands.send(Command::Search { query, req });
    }

    pub fn request_analyze(&mut self, path: PathBuf) {
        self.analyzer_path = path.clone();
        self.pane = Pane::Analyzer;
        self.req = self.req.wrapping_add(1);
        let req = self.req;
        let _ = self.commands.send(Command::Analyze { path, req });
    }

    pub fn visible_rows(&self) -> &[FileRow] {
        let tab = self.tab();
        if tab.search.is_empty() {
            &tab.rows
        } else {
            &tab.search_rows
        }
    }

    fn new_tab(&mut self) {
        self.tabs.push(Tab::at(self.places.home.clone(), true));
        self.current = self.tabs.len() - 1;
        self.pane = Pane::Files;
    }

    fn close_tab(&mut self) {
        if self.tabs.len() == 1 {
            return;
        }
        self.tabs.remove(self.current);
        if self.current >= self.tabs.len() {
            self.current = self.tabs.len() - 1;
        }
    }

    fn selection_paths(&self) -> Vec<PathBuf> {
        if self.pane == Pane::Analyzer {
            return self.analyzer_selected.iter().cloned().collect();
        }
        self.tab().selected.clone()
    }

    pub fn begin_delete(&mut self, permanent: bool) {
        let paths = self.selection_paths();
        if paths.is_empty() {
            self.status = self.tr.nothing_selected.into();
            return;
        }
        let (bytes, items, label) = self.delete_facts(&paths);
        self.delete_prompt = Some(DeletePrompt {
            paths,
            permanent,
            bytes,
            items,
            label,
        });
    }

    fn delete_facts(&self, paths: &[PathBuf]) -> (u64, u64, String) {
        let mut bytes = 0u64;
        let mut items = 0u64;
        let mut label = String::new();
        for path in paths {
            if label.is_empty() {
                label = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
            }
            if let Some((index, _)) = self
                .analyzer_rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.path == *path)
            {
                bytes = bytes.saturating_add(self.analyzer_rows[index].size);
                items = items.saturating_add(self.analyzer_counts.get(index).copied().unwrap_or(1));
                continue;
            }
            if let Some(row) = self.visible_rows().iter().find(|row| row.path == *path) {
                bytes = bytes.saturating_add(row.size);
                items = items.saturating_add(1);
                continue;
            }
            items = items.saturating_add(1);
        }
        if paths.len() > 1 {
            label = format!("{} (+{})", label, paths.len() - 1);
        }
        (bytes, items.max(1), label)
    }

    pub fn commit_delete(&mut self) {
        let Some(prompt) = self.delete_prompt.take() else {
            return;
        };
        self.status = self.tr.deleting.into();
        let command = if prompt.permanent {
            Command::DeletePermanent {
                paths: prompt.paths,
            }
        } else {
            Command::Trash {
                paths: prompt.paths,
            }
        };
        let _ = self.commands.send(command);
    }

    fn set_clipboard(&mut self, cut: bool) {
        let paths = self.selection_paths();
        if paths.is_empty() {
            self.status = self.tr.nothing_selected.into();
            return;
        }
        self.clipboard = Some(Clipboard {
            cut,
            paths: paths.clone(),
        });
        self.status = format!(
            "{} {} item(s)",
            if cut { "Cut" } else { "Copied" },
            paths.len()
        );
    }

    fn begin_rename(&mut self) {
        if self.tab().selected.len() == 1 {
            let path = self.tab().selected[0].clone();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.rename = Some((path, name));
        }
    }

    pub fn open_selection(&mut self) {
        let selected = self.tab().selected.clone();
        if selected.len() != 1 {
            return;
        }
        let path = selected[0].clone();
        let is_dir = self
            .visible_rows()
            .iter()
            .find(|row| row.path == path)
            .map(|row| row.is_dir);
        if is_dir.unwrap_or(path.is_dir()) {
            self.navigate(path);
        } else if let Err(err) = platform::open_path(&path) {
            self.status = err.to_string();
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible_rows().len() as isize;
        if len == 0 {
            return;
        }
        let current = self.tab().anchor.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len - 1) as usize;
        let path = self.visible_rows()[next].path.clone();
        let tab = self.tab_mut();
        tab.anchor = Some(next);
        tab.selected = vec![path];
    }

    pub fn select_row(&mut self, index: usize, modifiers: Modifiers) {
        let path = match self.visible_rows().get(index) {
            Some(row) => row.path.clone(),
            None => return,
        };
        let tab = self.tab_mut();
        if modifiers.command {
            if let Some(pos) = tab.selected.iter().position(|have| have == &path) {
                tab.selected.remove(pos);
            } else {
                tab.selected.push(path);
            }
            tab.anchor = Some(index);
        } else if modifiers.shift {
            let anchor = tab.anchor.unwrap_or(index);
            let (start, end) = if anchor < index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            // rows borrowed after mut tab; recompute from the stored vec
            let source_is_search = !tab.search.is_empty();
            let paths = if source_is_search {
                tab.search_rows[start..=end.min(tab.search_rows.len().saturating_sub(1))]
                    .iter()
                    .map(|row| row.path.clone())
                    .collect()
            } else {
                tab.rows[start..=end.min(tab.rows.len().saturating_sub(1))]
                    .iter()
                    .map(|row| row.path.clone())
                    .collect()
            };
            tab.selected = paths;
        } else {
            tab.selected = vec![path.clone()];
            tab.anchor = Some(index);
        }
        self.preview_selected();
    }

    pub fn mark_layout(&mut self, name: &str, rect: egui::Rect) {
        if self.layout_path.is_none() {
            return;
        }
        self.layout_log.push((
            name.to_string(),
            [rect.min.x, rect.min.y, rect.width(), rect.height()],
        ));
    }

    pub fn shot_step(&mut self, ctx: &egui::Context) {
        let Some(path) = self.shot_path.clone() else {
            return;
        };
        if self.shot_sent {
            return;
        }
        let ready = self
            .visible_rows()
            .iter()
            .any(|row| row.name == "hello.txt")
            || self.started.elapsed() > std::time::Duration::from_millis(800);
        if !ready {
            ctx.request_repaint();
            return;
        }
        self.shot_sent = true;
        let layout = self.layout_log.clone();
        let layout_path = self.layout_path.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
            egui::ScreenshotCallback::new(move |image| {
                save_screenshot(&path, &image);
                if let Some(layout_path) = layout_path {
                    let _ = std::fs::write(
                        layout_path,
                        serde_json::to_string(&layout).unwrap_or_default(),
                    );
                }
                std::process::exit(0);
            }),
        ));
        ctx.request_repaint();
    }

    pub fn request_icon(&mut self, path: &std::path::Path, px: u32) {
        if path.as_os_str().is_empty() {
            return;
        }
        let have = self.icon_px.get(path).copied().unwrap_or(0);
        if have >= px || !self.icon_asked.insert((path.to_path_buf(), px)) {
            return;
        }
        let _ = self.icon_tx.send(worker::IconJob {
            path: path.to_path_buf(),
            px,
        });
    }

    pub fn preview_selected(&mut self) {
        if self.tab().selected.len() == 1 {
            let path = self.tab().selected[0].clone();
            let _ = self.commands.send(Command::Preview { path });
        }
    }

    pub fn save_config(&self) {
        let config = Config {
            dark: self.dark,
            show_hidden: self.show_hidden,
            show_preview: self.show_preview,
            favorites: self.favorites.clone(),
        };
        let path = platform::config_dir().join("config.json");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&config) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn smoke_step(&mut self, ctx: &Context) {
        if self.smoke.is_none() {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        let has_hello = self.tab().rows.iter().any(|row| row.name == "hello.txt");
        let search = self.tab().search.clone();
        let has_nested = self
            .tab()
            .search_rows
            .iter()
            .any(|row| row.name.contains("nested"));
        let search_active = self.tab().search_active;
        let cells = self.analyzer_cells.len();
        let rows = self.tab().rows.len();
        let hits = self.tab().search_rows.len();
        if has_hello {
            if let Some(smoke) = self.smoke.as_mut() {
                smoke.saw_listing = true;
            }
        }
        let (saw_listing, saw_search, path) = {
            let smoke = self.smoke.as_ref().unwrap();
            (smoke.saw_listing, smoke.saw_search, smoke.path.clone())
        };
        if saw_listing && !saw_search {
            if search != "nested" {
                self.tab_mut().search = "nested".into();
                self.request_search();
            } else if has_nested {
                self.smoke.as_mut().unwrap().saw_search = true;
                self.request_analyze(path);
            } else if !search_active {
                self.request_search();
            }
        }
        if cells > 0 {
            if let Some(smoke) = self.smoke.as_mut() {
                if smoke.saw_search {
                    smoke.saw_analyze = true;
                }
            }
        }
        let done = self
            .smoke
            .as_ref()
            .map(|smoke| smoke.saw_listing && smoke.saw_search && smoke.saw_analyze)
            .unwrap_or(false);
        if done {
            println!("smoke ok");
            self.smoke = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.started.elapsed() > std::time::Duration::from_secs(20) {
            let smoke = self.smoke.as_ref().unwrap();
            eprintln!(
                "smoke failed listing={} search={} analyze={} rows={rows} hits={hits} cells={cells} status={}",
                smoke.saw_listing, smoke.saw_search, smoke.saw_analyze, self.status
            );
            std::process::exit(1);
        }
    }
}

fn install_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "fluent".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fluent-icons.ttf"
        ))),
    );
    fonts
        .families
        .entry(egui::FontFamily::Name("fluent".into()))
        .or_default()
        .push("fluent".into());
    #[cfg(windows)]
    for file in [
        r"C:\Windows\Fonts\segoeui.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\SegUIVar.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(file) {
            fonts.font_data.insert(
                "segoe".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                proportional.insert(0, "segoe".into());
            }
            break;
        }
    }
    #[cfg(windows)]
    for file in [
        r"C:\Windows\Fonts\segmdl2.ttf",
        r"C:\Windows\Fonts\SegoeIcons.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(file) {
            fonts.font_data.insert(
                "mdl2".into(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Name("mdl2".into()))
                .or_default()
                .push("mdl2".into());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.selection.bg_fill = egui::Color32::from_rgb(0, 112, 203);
    visuals.window_corner_radius = egui::CornerRadius::same(8);
    ctx.set_visuals(visuals);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.style_mut_of(theme, |style| {
            style.animation_time = 0.12;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        });
    }
}

fn load_config() -> Config {
    let path = platform::config_dir().join("config.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn format_modified_lang(unix: u64, lang: crate::i18n::Lang) -> String {
    use chrono::TimeZone;
    if unix == 0 {
        return String::new();
    }
    let pattern = match lang {
        crate::i18n::Lang::Fr => "%d/%m/%Y %H:%M",
        crate::i18n::Lang::En => "%Y-%m-%d %H:%M",
    };
    match chrono::Local.timestamp_opt(unix as i64, 0) {
        chrono::MappedLocalTime::Single(time) => time.format(pattern).to_string(),
        _ => String::new(),
    }
}

pub fn format_bytes(bytes: u64, lang: crate::i18n::Lang) -> String {
    let french = lang == crate::i18n::Lang::Fr;
    let units: [&str; 5] = if french {
        ["o", "Ko", "Mo", "Go", "To"]
    } else {
        ["B", "KB", "MB", "GB", "TB"]
    };
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    let text = if unit == 0 {
        format!("{} {}", bytes, units[0])
    } else if value >= 10.0 {
        format!("{value:.0} {}", units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    };
    if french {
        text.replace('.', ",")
    } else {
        text
    }
}

pub fn format_free(free: u64, total: u64, lang: crate::i18n::Lang) -> String {
    match lang {
        crate::i18n::Lang::Fr => format!(
            "{} libres sur {}",
            format_bytes(free, lang),
            format_bytes(total, lang)
        ),
        crate::i18n::Lang::En => format!(
            "{} free of {}",
            format_bytes(free, lang),
            format_bytes(total, lang)
        ),
    }
}

fn save_screenshot(path: &std::path::Path, image: &egui::ColorImage) {
    let mut bytes = Vec::with_capacity(image.pixels.len() * 4);
    for pixel in &image.pixels {
        bytes.extend_from_slice(&pixel.to_array());
    }
    let _ = image::save_buffer(
        path,
        &bytes,
        image.size[0] as u32,
        image.size[1] as u32,
        image::ColorType::Rgba8,
    );
}

fn paths_match(left: &std::path::Path, right: &std::path::Path) -> bool {
    use std::path::Component;
    let normalize = |path: &std::path::Path| {
        path.components()
            .filter(|component| !matches!(component, Component::CurDir))
            .map(|component| component.as_os_str().to_os_string())
            .collect::<Vec<_>>()
    };
    normalize(left) == normalize(right)
}
