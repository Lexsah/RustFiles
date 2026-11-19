//! Immediate-mode layout. Every click turns into a worker command; nothing here
//! reads the disk.

use egui::{
    Align, Color32, CornerRadius, Layout, RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, Vec2,
};

use crate::app::{apply_theme, format_modified, type_name, Pane, RustFilesApp};
use crate::nav;
use crate::worker::{Command, MapCell};
use rustfiles_core::{format_size, SortColumn};

impl RustFilesApp {
    pub fn draw(&mut self, ui: &mut Ui) {
        apply_theme(ui.ctx(), self.dark);
        self.draw_tabs(ui);
        self.draw_toolbar(ui);
        self.draw_status(ui);
        self.draw_sidebar(ui);
        if self.show_preview && self.pane == Pane::Files {
            self.draw_preview(ui);
        }
        egui::CentralPanel::default().show(ui, |ui| match self.pane {
            Pane::Files => self.draw_files(ui),
            Pane::Analyzer => self.draw_analyzer(ui),
        });
        self.draw_dialogs(ui);
    }

    fn draw_tabs(&mut self, ui: &mut Ui) {
        egui::Panel::top("tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut close = None;
                let mut select = None;
                for (index, tab) in self.tabs.iter().enumerate() {
                    let title = tab
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| tab.path.display().to_string());
                    let label = if title.chars().count() > 18 {
                        format!("{}…", title.chars().take(18).collect::<String>())
                    } else {
                        title
                    };
                    if ui.selectable_label(index == self.current, label).clicked() {
                        select = Some(index);
                    }
                    if ui.small_button("×").clicked() {
                        close = Some(index);
                    }
                }
                if ui.button("+").clicked() {
                    self.new_tab_public();
                }
                if let Some(index) = select {
                    self.current = index;
                    self.pane = Pane::Files;
                }
                if let Some(index) = close {
                    self.current = index;
                    self.close_tab_public();
                }
            });
        });
    }

    fn draw_toolbar(&mut self, ui: &mut Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled_ui(self.tab().hist_pos > 0, |ui| {
                    if ui.button("←").on_hover_text("Back (Alt+Left)").clicked() {
                        self.perform(crate::nav::Action::Back);
                    }
                });
                ui.add_enabled_ui(self.tab().hist_pos + 1 < self.tab().history.len(), |ui| {
                    if ui
                        .button("→")
                        .on_hover_text("Forward (Alt+Right)")
                        .clicked()
                    {
                        self.perform(crate::nav::Action::Forward);
                    }
                });
                if ui.button("↑").on_hover_text("Up (Alt+Up)").clicked() {
                    self.perform(crate::nav::Action::Up);
                }
                self.draw_crumbs(ui);
                if ui
                    .button(if self.tab().details {
                        "Grid"
                    } else {
                        "Details"
                    })
                    .clicked()
                {
                    self.tab_mut().details = !self.tab().details;
                }
                if ui.button("New folder").clicked() {
                    self.perform(crate::nav::Action::NewFolder);
                }
                if ui.button("Analyzer").clicked() {
                    self.perform(crate::nav::Action::Analyzer);
                }
                if ui
                    .button("Reindex")
                    .on_hover_text("Rebuild the filename index")
                    .clicked()
                {
                    self.indexing = true;
                    let _ = self.commands.send(crate::worker::Command::Rescan);
                }
                let mut query = self.tab().search.clone();
                let editor = TextEdit::singleline(&mut query)
                    .hint_text("Search filenames")
                    .desired_width(220.0);
                let response = ui.add(editor);
                if self.focus_search {
                    response.request_focus();
                    self.focus_search = false;
                }
                if response.changed() {
                    self.tab_mut().search = query;
                    self.request_search();
                }
            });
        });
    }

    fn draw_crumbs(&mut self, ui: &mut Ui) {
        if let Some(mut text) = self.path_edit.clone() {
            let response = ui.add(TextEdit::singleline(&mut text).desired_width(360.0));
            if self.focus_path {
                response.request_focus();
                self.focus_path = false;
            }
            if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.navigate(std::path::PathBuf::from(&text));
                self.path_edit = None;
            } else if response.changed() {
                self.path_edit = Some(text);
            }
            return;
        }
        ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (label, path) in nav::breadcrumbs(&self.tab().path) {
                    if ui.button(label).clicked() {
                        self.navigate(path);
                        return;
                    }
                    ui.label("›");
                }
                if ui
                    .small_button("✎")
                    .on_hover_text("Edit path (Ctrl+L)")
                    .clicked()
                {
                    self.perform(crate::nav::Action::FocusPath);
                }
            });
        });
    }

    fn draw_sidebar(&mut self, ui: &mut Ui) {
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    ui.label(RichText::new("Favorites").small().weak());
                    let favorites = self.favorites.clone();
                    for path in favorites {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        let selected = self.tab().path == path;
                        if ui.selectable_label(selected, name).clicked() {
                            self.navigate(path);
                        }
                    }
                    if ui.button("Pin current folder").clicked() {
                        let path = self.tab().path.clone();
                        if !self.favorites.contains(&path) {
                            self.favorites.push(path);
                            self.save_config();
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("Drives").small().weak());
                    let volumes = self.volumes.clone();
                    for volume in volumes {
                        let label = if volume.label.is_empty() || volume.label == volume.fs_name {
                            volume.mount.clone()
                        } else {
                            format!("{}  {}", volume.mount, volume.label)
                        };
                        if ui
                            .selectable_label(
                                self.tab().path == std::path::Path::new(&volume.mount),
                                label,
                            )
                            .clicked()
                        {
                            self.navigate(std::path::PathBuf::from(volume.mount));
                        }
                    }
                });
            });
    }

    fn draw_status(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let count = self.visible_rows().len();
                let selected = self.tab().selected.len();
                ui.label(format!("{count} items"));
                if selected > 0 {
                    ui.label(format!("{selected} selected"));
                }
                if self.tab().from_index {
                    ui.label("cached index");
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(if self.dark { "Light" } else { "Dark" })
                        .clicked()
                    {
                        self.dark = !self.dark;
                        self.save_config();
                    }
                    if ui
                        .button(if self.show_hidden {
                            "Hide hidden"
                        } else {
                            "Show hidden"
                        })
                        .clicked()
                    {
                        self.show_hidden = !self.show_hidden;
                        self.save_config();
                        self.request_list();
                    }
                    if ui.button("?").clicked() {
                        self.help = !self.help;
                    }
                    let indexed = self
                        .progress
                        .files
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .max(self.indexed);
                    let label = if self.indexing {
                        format!("Indexing… {indexed}")
                    } else {
                        format!("{indexed} indexed")
                    };
                    ui.label(label);
                    ui.label(&self.status);
                });
            });
        });
    }

    fn draw_files(&mut self, ui: &mut Ui) {
        let searching = !self.tab().search.is_empty();
        if searching {
            ui.label(format!("Search results for \"{}\"", self.tab().search));
        }
        if self.tab().details {
            self.draw_details(ui);
        } else {
            self.draw_grid(ui);
        }
    }

    fn draw_details(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let name_w = (width - 360.0).max(140.0);
        ui.horizontal(|ui| {
            header(ui, "Name", name_w, SortColumn::Name, self);
            header(ui, "Date modified", 150.0, SortColumn::Modified, self);
            header(ui, "Type", 90.0, SortColumn::Type, self);
            header(ui, "Size", 90.0, SortColumn::Size, self);
        });
        let rows_len = self.visible_rows().len();
        if rows_len == 0 {
            ui.add_space(12.0);
            ui.label(if self.tab().search.is_empty() {
                "This folder is empty."
            } else {
                "No matching files."
            });
            return;
        }
        let mut clicked = None;
        let mut dbl = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 22.0, rows_len, |ui, range| {
                for index in range {
                    let row = &self.visible_rows()[index];
                    let selected = self.tab().selected.iter().any(|path| path == &row.path);
                    let name = row.name.clone();
                    let modified = format_modified(row.modified);
                    let kind = type_name(row);
                    let size = if row.is_dir && row.size == 0 {
                        "—".into()
                    } else {
                        format_size(row.size)
                    };
                    let is_dir = row.is_dir;
                    ui.horizontal(|ui| {
                        let response =
                            ui.add_sized([name_w, 20.0], egui::Button::selectable(selected, name));
                        if response.clicked() {
                            clicked = Some((index, ui.input(|input| input.modifiers)));
                        }
                        if response.double_clicked() {
                            dbl = Some(index);
                        }
                        ui.add_sized([150.0, 20.0], egui::Label::new(modified));
                        ui.add_sized([90.0, 20.0], egui::Label::new(kind));
                        ui.add_sized([90.0, 20.0], egui::Label::new(size));
                        let _ = is_dir;
                    });
                }
            });
        if let Some((index, mods)) = clicked {
            self.select_row(index, mods);
        }
        if let Some(index) = dbl {
            self.select_row(index, egui::Modifiers::NONE);
            self.open_selection();
        }
    }

    fn draw_grid(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let cols = ((width / 104.0).floor() as usize).max(1);
        let len = self.visible_rows().len();
        if len == 0 {
            ui.label("This folder is empty.");
            return;
        }
        let grid_rows = len.div_ceil(cols);
        let mut clicked = None;
        let mut dbl = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 86.0, grid_rows, |ui, range| {
                for row_index in range {
                    ui.horizontal(|ui| {
                        for col in 0..cols {
                            let index = row_index * cols + col;
                            if index >= len {
                                break;
                            }
                            let row = &self.visible_rows()[index];
                            let selected = self.tab().selected.iter().any(|path| path == &row.path);
                            let name = row.name.clone();
                            let is_dir = row.is_dir;
                            let response =
                                ui.allocate_response(Vec2::new(100.0, 80.0), Sense::click());
                            let color = if selected {
                                ui.visuals().selection.bg_fill
                            } else if response.hovered() {
                                ui.visuals().widgets.hovered.bg_fill
                            } else {
                                Color32::TRANSPARENT
                            };
                            ui.painter()
                                .rect_filled(response.rect, CornerRadius::same(6), color);
                            let icon = if is_dir {
                                Color32::from_rgb(232, 184, 74)
                            } else {
                                Color32::from_rgb(121, 162, 217)
                            };
                            let icon_rect = egui::Rect::from_center_size(
                                response.rect.center() - Vec2::new(0.0, 12.0),
                                Vec2::splat(28.0),
                            );
                            ui.painter()
                                .rect_filled(icon_rect, CornerRadius::same(4), icon);
                            ui.painter().text(
                                response.rect.center() + Vec2::new(0.0, 24.0),
                                egui::Align2::CENTER_CENTER,
                                truncate(&name, 14),
                                egui::FontId::proportional(12.0),
                                ui.visuals().text_color(),
                            );
                            if response.clicked() {
                                clicked = Some((index, ui.input(|input| input.modifiers)));
                            }
                            if response.double_clicked() {
                                dbl = Some(index);
                            }
                        }
                    });
                }
            });
        if let Some((index, mods)) = clicked {
            self.select_row(index, mods);
        }
        if let Some(index) = dbl {
            self.select_row(index, egui::Modifiers::NONE);
            self.open_selection();
        }
    }

    fn draw_preview(&mut self, ui: &mut Ui) {
        egui::Panel::right("preview")
            .resizable(true)
            .default_size(280.0)
            .show(ui, |ui| {
                ui.heading("Preview");
                let selected = self.tab().selected.clone();
                if selected.len() != 1 {
                    ui.label(if selected.is_empty() {
                        "Select a file."
                    } else {
                        "Multiple items selected."
                    });
                    return;
                }
                let path = &selected[0];
                ui.label(
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
                ui.label(path.display().to_string());
                if let Some(texture) = &self.preview_image {
                    if self.preview_path == *path {
                        ui.image((texture.id(), texture.size_vec2()));
                    }
                }
                if self.preview_path == *path && !self.preview_text.is_empty() {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.label(RichText::new(&self.preview_text).monospace());
                    });
                } else if !self.preview_note.is_empty() && self.preview_path == *path {
                    ui.label(&self.preview_note);
                }
            });
    }

    fn draw_analyzer(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("Up").clicked() {
                if let Some(parent) = nav::parent_dir(&self.analyzer_path) {
                    self.request_analyze(parent);
                }
            }
            if ui.button("Back to files").clicked() {
                self.pane = Pane::Files;
            }
            if ui
                .button(if self.sunburst { "Treemap" } else { "Sunburst" })
                .clicked()
            {
                self.sunburst = !self.sunburst;
            }
            ui.heading(format!(
                "{}   {}",
                self.analyzer_path.display(),
                format_size(self.analyzer_total)
            ));
        });
        ui.separator();
        let cells = self.analyzer_cells.clone();
        let rows = self.analyzer_rows.clone();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(280.0);
                ScrollArea::vertical().show(ui, |ui| {
                    for row in &rows {
                        let label = format!(
                            "{}   {}",
                            row.name,
                            if row.size == 0 {
                                "—".into()
                            } else {
                                format_size(row.size)
                            }
                        );
                        if ui.selectable_label(false, label).clicked() && row.is_dir {
                            self.request_analyze(row.path.clone());
                            return;
                        }
                    }
                });
            });
            ui.vertical(|ui| {
                let available = ui.available_size();
                let (rect, response) = ui.allocate_exact_size(available, Sense::click());
                if self.sunburst {
                    paint_sunburst(ui, rect, &cells);
                } else {
                    paint_treemap(ui, rect, &cells);
                }
                if let Some(pointer) = response.hover_pos() {
                    if let Some(cell) = hit_cell(&cells, rect, pointer, self.sunburst) {
                        response.clone().on_hover_text(format!(
                            "{}\n{}",
                            cell.name,
                            format_size(cell.size)
                        ));
                    }
                }
                if response.clicked() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        if let Some(cell) = hit_cell(&cells, rect, pointer, self.sunburst) {
                            if cell.is_dir {
                                self.request_analyze(cell.path.clone());
                            }
                        }
                    }
                }
            });
        });
    }

    fn draw_dialogs(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if self.help {
            egui::Window::new("Keyboard shortcuts")
                .open(&mut self.help)
                .show(&ctx, |ui| {
                    ui.label("Ctrl+T new tab    Ctrl+W close tab    Ctrl+Tab next tab");
                    ui.label("Alt+Left / Right back and forward    Alt+Up or Backspace parent");
                    ui.label("Ctrl+L path    Ctrl+F search    F5 refresh    Enter open");
                    ui.label("Ctrl+C / X / V copy, cut, paste    Ctrl+Shift+N new folder");
                    ui.label("F2 rename    Delete trash    Shift+Delete permanent delete");
                    ui.label("Ctrl+A select all    F1 this list");
                });
        }
        if let Some((path, mut name)) = self.rename.clone() {
            let mut open = true;
            let mut commit = false;
            egui::Window::new("Rename")
                .open(&mut open)
                .show(&ctx, |ui| {
                    ui.label(path.display().to_string());
                    let response = ui.add(TextEdit::singleline(&mut name).desired_width(280.0));
                    response.request_focus();
                    if ui.button("Rename").clicked()
                        || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        commit = true;
                    }
                });
            if commit {
                let _ = self.commands.send(Command::Rename {
                    path,
                    new_name: name,
                });
                self.rename = None;
            } else if open {
                self.rename = Some((path, name));
            } else {
                self.rename = None;
            }
        }
        if !self.confirm_delete.is_empty() {
            let paths = self.confirm_delete.clone();
            let mut open = true;
            let mut confirm = false;
            egui::Window::new("Delete permanently")
                .open(&mut open)
                .show(&ctx, |ui| {
                    ui.label(format!(
                        "Delete {} item(s) without using the trash?",
                        paths.len()
                    ));
                    if ui.button("Delete").clicked() {
                        confirm = true;
                    }
                });
            if confirm {
                let _ = self.commands.send(Command::DeletePermanent { paths });
                self.confirm_delete.clear();
            } else if !open {
                self.confirm_delete.clear();
            }
        }
    }

    fn new_tab_public(&mut self) {
        self.perform(crate::nav::Action::NewTab);
    }

    fn close_tab_public(&mut self) {
        self.perform(crate::nav::Action::CloseTab);
    }
}

fn header(ui: &mut Ui, title: &str, width: f32, column: SortColumn, app: &mut RustFilesApp) {
    let active = app.tab().sort == column;
    let arrow = if !active {
        ""
    } else if app.tab().ascending {
        " ↑"
    } else {
        " ↓"
    };
    if ui
        .add_sized(
            [width, 18.0],
            egui::Button::new(format!("{title}{arrow}")).frame(false),
        )
        .clicked()
    {
        let tab = app.tab_mut();
        if tab.sort == column {
            tab.ascending = !tab.ascending;
        } else {
            tab.sort = column;
            tab.ascending = column != SortColumn::Size && column != SortColumn::Modified;
        }
        let sort = tab.sort;
        let ascending = tab.ascending;
        let searching = !tab.search.is_empty();
        if searching {
            sort_rows_in(&mut tab.search_rows, sort, ascending);
        } else {
            sort_rows_in(&mut tab.rows, sort, ascending);
        }
    }
}

fn sort_rows_in(rows: &mut [rustfiles_core::FileRow], column: SortColumn, ascending: bool) {
    rustfiles_core::sort_rows(rows, column, ascending, true);
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!(
            "{}…",
            text.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

fn paint_treemap(ui: &Ui, rect: egui::Rect, cells: &[MapCell]) {
    let painter = ui.painter();
    for cell in cells {
        let visual = egui::Rect::from_min_size(
            rect.min + Vec2::new(cell.rect.x * rect.width(), cell.rect.y * rect.height()),
            Vec2::new(cell.rect.w * rect.width(), cell.rect.h * rect.height()),
        );
        if visual.width() < 2.0 || visual.height() < 2.0 {
            continue;
        }
        painter.rect_filled(
            visual.shrink(1.0),
            CornerRadius::same(2),
            color_for(&cell.name, ui.visuals().dark_mode),
        );
        if visual.width() > 48.0 && visual.height() > 18.0 {
            painter.text(
                visual.left_top() + Vec2::new(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                truncate(&cell.name, 24),
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
        }
    }
}

fn paint_sunburst(ui: &Ui, rect: egui::Rect, cells: &[MapCell]) {
    let painter = ui.painter();
    let center = rect.center();
    let outer = rect.width().min(rect.height()) * 0.46;
    let inner = outer * 0.42;
    for cell in cells {
        paint_arc(
            painter,
            center,
            inner,
            outer,
            cell.start,
            cell.end,
            color_for(&cell.name, ui.visuals().dark_mode),
        );
    }
    painter.circle_filled(center, inner * 0.92, ui.visuals().window_fill);
}

fn paint_arc(
    painter: &egui::Painter,
    center: egui::Pos2,
    inner: f32,
    outer: f32,
    start: f32,
    end: f32,
    color: Color32,
) {
    let span = end - start;
    if span <= 0.0 {
        return;
    }
    if span > std::f32::consts::PI {
        let mid = start + span / 2.0;
        paint_arc(painter, center, inner, outer, start, mid, color);
        paint_arc(painter, center, inner, outer, mid, end, color);
        return;
    }
    let steps = ((span * 18.0).ceil() as i32).max(2);
    let mut points = Vec::new();
    for step in 0..=steps {
        let t = start + span * (step as f32 / steps as f32);
        points.push(center + Vec2::new(outer * t.cos(), outer * t.sin()));
    }
    for step in (0..=steps).rev() {
        let t = start + span * (step as f32 / steps as f32);
        points.push(center + Vec2::new(inner * t.cos(), inner * t.sin()));
    }
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        Stroke::new(1.0, Color32::from_black_alpha(80)),
    ));
}

fn hit_cell(
    cells: &[MapCell],
    rect: egui::Rect,
    pointer: egui::Pos2,
    sunburst: bool,
) -> Option<&MapCell> {
    if sunburst {
        let center = rect.center();
        let delta = pointer - center;
        let radius = delta.length();
        let outer = rect.width().min(rect.height()) * 0.46;
        let inner = outer * 0.42;
        if radius < inner || radius > outer {
            return None;
        }
        let mut angle = delta.y.atan2(delta.x);
        if angle < 0.0 {
            angle += std::f32::consts::TAU;
        }
        cells
            .iter()
            .find(|cell| angle >= cell.start && angle < cell.end)
    } else {
        let local = egui::Pos2::new(
            (pointer.x - rect.min.x) / rect.width(),
            (pointer.y - rect.min.y) / rect.height(),
        );
        cells.iter().find(|cell| {
            local.x >= cell.rect.x
                && local.y >= cell.rect.y
                && local.x <= cell.rect.x + cell.rect.w
                && local.y <= cell.rect.y + cell.rect.h
        })
    }
}

fn color_for(name: &str, dark: bool) -> Color32 {
    let mut hash = 2_166_136_261u32;
    for byte in name.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    let hue = (hash % 360) as f32;
    let (sat, value) = if dark { (0.46, 0.70) } else { (0.52, 0.82) };
    let (r, g, b) = hsv(hue, sat, value);
    Color32::from_rgb(r, g, b)
}

fn hsv(hue: f32, sat: f32, value: f32) -> (u8, u8, u8) {
    let c = value * sat;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = value - c;
    let (r, g, b) = match hue as i32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}
